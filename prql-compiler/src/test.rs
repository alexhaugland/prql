//! Simple tests for "this PRQL creates this SQL" go here.
// use super::*;
use crate::{parser::parse, sql};
use insta::{assert_display_snapshot, assert_snapshot};

pub fn compile(prql: &str) -> Result<String, crate::ErrorMessages> {
    crate::parser::parse(prql)
        .and_then(crate::semantic::resolve)
        .and_then(|rq| sql::compile(rq, sql::Options::default().no_signature().some()))
        .map_err(crate::error::downcast)
        .map_err(|e| e.composed("", prql, false))
}

#[test]
fn test_stdlib() {
    assert_snapshot!(compile(r###"
    from employees
    aggregate (
        [salary_usd = min salary]
    )
    "###).unwrap(),
        @r###"
    SELECT
      MIN(salary) AS salary_usd
    FROM
      employees
    "###
    );

    assert_snapshot!(compile(r###"
    from employees
    aggregate (
        [salary_usd = (round 2 salary)]
    )
    "###).unwrap(),
        @r###"
    SELECT
      ROUND(salary, 2) AS salary_usd
    FROM
      employees
    "###
    );
}

#[test]
fn json_of_test() {
    let json = crate::prql_to_pl("from employees | take 10")
        .and_then(crate::json::from_pl)
        .unwrap();
    // Since the AST is so in flux right now just test that the brackets are present
    assert_eq!(json.chars().next().unwrap(), '[');
    assert_eq!(json.chars().nth(json.len() - 1).unwrap(), ']');
}

#[test]
fn test_precedence() {
    assert_display_snapshot!((compile(r###"
    from x
    derive [
        n = a + b,
        r = a/n,
    ]
    select temp_c = (temp - 32) / 1.8
    "###).unwrap()), @r###"
    SELECT
      (temp - 32) / 1.8 AS temp_c
    FROM
      x
    "###);

    assert_display_snapshot!((compile(r###"
    func add x y -> x + y

    from numbers
    derive [sum_1 = a + b, sum_2 = add a b]
    select [result = c * sum_1 + sum_2]
    "###).unwrap()), @r###"
    SELECT
      c * (a + b) + a + b AS result
    FROM
      numbers
    "###);

    assert_display_snapshot!((compile(r###"
    from numbers
    derive [g = -a]
    select a * g
    "###).unwrap()), @r###"
    SELECT
      a * - a
    FROM
      numbers
    "###);

    assert_display_snapshot!((compile(r###"
    from numbers
    select negated_is_null = (!a) == null
    "###).unwrap()), @r###"
    SELECT
      (NOT a) IS NULL AS negated_is_null
    FROM
      numbers
    "###);

    assert_display_snapshot!((compile(r###"
    from numbers
    select is_not_null = !(a == null)
    "###).unwrap()), @r###"
    SELECT
      NOT a IS NULL AS is_not_null
    FROM
      numbers
    "###);

    assert_display_snapshot!(compile(
        r###"
    from numbers
    select (a + b) == null
    "###
    ).unwrap(), @r###"
    SELECT
      a + b IS NULL
    FROM
      numbers
    "###
    );

    assert_display_snapshot!(compile(
        r###"
        from numbers
        derive x = (y - z)
        select [
        c - (a + b),
        c + (a - b),
        c + a - b,
        c + a + b,
        (c + a) - b,
        ((c - d) - (a - b)),
        ((c + d) + (a - b)),
        +x,
        -x,
        ]
        "###
        ).unwrap(), @r###"
    SELECT
      c - (a + b),
      c + a - b,
      c + a - b,
      c + a + b,
      c + a - b,
      c - d - (a - b),
      c + d + a - b,
      y - z AS x,
      -(y - z)
    FROM
      numbers
    "###
    );
}

#[test]
fn test_pipelines() {
    assert_display_snapshot!((compile(r###"
    from employees
    group dept (take 1)
    "###).unwrap()), @r###"
    SELECT
      DISTINCT *
    FROM
      employees
    "###);
}

#[test]
fn test_concat() {
    assert_display_snapshot!(compile(r###"
    from employees
    concat managers
    "###).unwrap(), @r###"
    (
      SELECT
        *
      FROM
        employees
    )
    UNION
    ALL
    SELECT
      *
    FROM
      managers
    "###);

    assert_display_snapshot!(compile(r###"
    from employees
    derive [name, cost = salary]
    take 3
    concat (
        from employees
        derive [name, cost = salary + bonuses]
        take 10
    )
    "###).unwrap(), @r###"
    WITH table_1 AS (
      SELECT
        *,
        name,
        salary + bonuses AS cost
      FROM
        employees
      LIMIT
        10
    ) (
      SELECT
        *,
        name,
        salary AS cost
      FROM
        employees
      LIMIT
        3
    )
    UNION
    ALL
    SELECT
      *
    FROM
      table_1 AS table_0
    "###);

    assert_display_snapshot!(compile(r###"
    from employees
    union managers
    "###).unwrap(), @r###"
    (
      SELECT
        *
      FROM
        employees
    )
    UNION
    DISTINCT
    SELECT
      *
    FROM
      managers
    "###);

    assert_display_snapshot!(compile(r###"
    from employees
    concat managers
    union all_employees_of_some_other_company
    "###).unwrap(), @r###"
    WITH table_1 AS (
      (
        SELECT
          *
        FROM
          employees
      )
      UNION
      ALL
      SELECT
        *
      FROM
        managers
    ) (
      SELECT
        *
      FROM
        table_1
    )
    UNION
    DISTINCT
    SELECT
      *
    FROM
      all_employees_of_some_other_company
    "###);
}

#[test]
fn test_rn_ids_are_unique() {
    assert_display_snapshot!((compile(r###"
    from y_orig
    group [y_id] (
        take 2 # take 1 uses `distinct` instead of partitioning, which might be a separate bug
    )
    group [x_id] (
        take 3
    )
    "###).unwrap()), @r###"
    WITH table_1 AS (
      SELECT
        *,
        ROW_NUMBER() OVER (PARTITION BY y_id) AS _expr_0
      FROM
        y_orig
    ),
    table_2 AS (
      SELECT
        *,
        ROW_NUMBER() OVER (PARTITION BY x_id) AS _expr_1
      FROM
        table_1
      WHERE
        _expr_0 <= 2
    )
    SELECT
      *
    FROM
      table_2
    WHERE
      _expr_1 <= 3
    "###);
}

#[test]
fn test_quoting() {
    // GH-#822
    assert_display_snapshot!((compile(r###"
prql sql_dialect:postgres
table UPPER = (
from lower
)
from UPPER
join `some_schema.tablename` [==id]
derive `from` = 5
    "###).unwrap()), @r###"
    WITH "UPPER" AS (
      SELECT
        *
      FROM
        lower
    )
    SELECT
      "UPPER".*,
      some_schema.tablename.*,
      5 AS "from"
    FROM
      "UPPER"
      JOIN some_schema.tablename ON "UPPER".id = some_schema.tablename.id
    "###);

    // GH-#852
    assert_display_snapshot!((compile(r###"
prql sql_dialect:bigquery
from `db.schema.table`
join `db.schema.table2` [==id]
join c = `db.schema.t-able` [`db.schema.table`.id == c.id]
    "###).unwrap()), @r###"
    SELECT
      `db.schema.table`.*,
      `db.schema.table2`.*,
      c.*
    FROM
      `db.schema.table`
      JOIN `db.schema.table2` ON `db.schema.table`.id = `db.schema.table2`.id
      JOIN `db.schema.t-able` AS c ON `db.schema.table`.id = c.id
    "###);

    assert_display_snapshot!((compile(r###"
from table
select `first name`
    "###).unwrap()), @r###"
    SELECT
      "first name"
    FROM
      table
    "###);
}

#[test]
fn test_sorts() {
    assert_display_snapshot!((compile(r###"
    from invoices
    sort [issued_at, -amount, +num_of_articles]
    "###
    ).unwrap()), @r###"
    SELECT
      *
    FROM
      invoices
    ORDER BY
      issued_at,
      amount DESC,
      num_of_articles
    "###);

    assert_display_snapshot!((compile(r###"
    from x
    derive somefield = "something"
    sort [somefield]
    select [renamed = somefield]
    "###
    ).unwrap()), @r###"
    WITH table_1 AS (
      SELECT
        'something' AS renamed,
        'something' AS _expr_0
      FROM
        x
      ORDER BY
        _expr_0
    )
    SELECT
      renamed
    FROM
      table_1
    "###);
}

#[test]
fn test_numbers() {
    let query = r###"
    from numbers
    select [
        x = 5,
        y = 5.0,
        z = 5.00,
    ]
    "###;

    assert_display_snapshot!((compile(query).unwrap()), @r###"
    SELECT
      5 AS x,
      5.0 AS y,
      5.0 AS z
    FROM
      numbers
    "###);
}

#[test]
fn test_ranges() {
    let query = r###"
    from employees
    filter (age | in 18..40)
    "###;

    assert_display_snapshot!((compile(query).unwrap()), @r###"
    SELECT
      *
    FROM
      employees
    WHERE
      age BETWEEN 18
      AND 40
    "###);

    let query = r###"
    from employees
    filter (age | in ..40)
    "###;

    assert!(compile(query).is_err());

    let query = r###"
    from events
    filter (date | in @1776-07-04..@1787-09-17)
    "###;

    assert_display_snapshot!((compile(query).unwrap()), @r###"
    SELECT
      *
    FROM
      events
    WHERE
      date BETWEEN DATE '1776-07-04'
      AND DATE '1787-09-17'
    "###);
}

#[test]
fn test_interval() {
    let query = r###"
    from projects
    derive first_check_in = start + 10days
    "###;

    assert_display_snapshot!((compile(query).unwrap()), @r###"
    SELECT
      *,
      start + INTERVAL 10 DAY AS first_check_in
    FROM
      projects
    "###);
}

#[test]
fn test_dates() {
    assert_display_snapshot!((compile(r###"
    from to_do_empty_table
    derive [
        date = @2011-02-01,
        timestamp = @2011-02-01T10:00,
        time = @14:00,
        # datetime = @2011-02-01T10:00<datetime>,
    ]
    "###).unwrap()), @r###"
    SELECT
      *,
      DATE '2011-02-01' AS date,
      TIMESTAMP '2011-02-01T10:00' AS timestamp,
      TIME '14:00' AS time
    FROM
      to_do_empty_table
    "###);
}

#[test]
fn test_window_functions_00() {
    assert_display_snapshot!((compile(r###"
    from employees
    group last_name (
        derive count
    )
    "###).unwrap()), @r###"
    SELECT
      *,
      COUNT(*) OVER (PARTITION BY last_name)
    FROM
      employees
    "###);
}

#[test]
fn test_window_functions_02() {
    let query = r###"
    from co=cust_order
    join ol=order_line [==order_id]
    derive [
        order_month = s"TO_CHAR({co.order_date}, '%Y-%m')",
        order_day = s"TO_CHAR({co.order_date}, '%Y-%m-%d')",
    ]
    group [order_month, order_day] (
        aggregate [
            num_orders = s"COUNT(DISTINCT {co.order_id})",
            num_books = count non_null:ol.book_id,
            total_price = sum ol.price,
        ]
    )
    group [order_month] (
        sort order_day
        window expanding:true (
            derive [running_total_num_books = sum num_books]
        )
    )
    sort order_day
    derive [num_books_last_week = lag 7 num_books]
    "###;

    assert_display_snapshot!((compile(query).unwrap()), @r###"
    WITH table_1 AS (
      SELECT
        TO_CHAR(co.order_date, '%Y-%m') AS order_month,
        TO_CHAR(co.order_date, '%Y-%m-%d') AS order_day,
        COUNT(DISTINCT co.order_id) AS num_orders,
        COUNT(ol.book_id) AS num_books,
        SUM(ol.price) AS total_price
      FROM
        cust_order AS co
        JOIN order_line AS ol ON co.order_id = ol.order_id
      GROUP BY
        TO_CHAR(co.order_date, '%Y-%m'),
        TO_CHAR(co.order_date, '%Y-%m-%d')
    )
    SELECT
      order_month,
      order_day,
      num_orders,
      num_books,
      total_price,
      SUM(num_books) OVER (
        PARTITION BY order_month
        ORDER BY
          order_day ROWS BETWEEN UNBOUNDED PRECEDING
          AND CURRENT ROW
      ) AS running_total_num_books,
      LAG(num_books, 7) OVER (
        ORDER BY
          order_day ROWS BETWEEN UNBOUNDED PRECEDING
          AND UNBOUNDED FOLLOWING
      ) AS num_books_last_week
    FROM
      table_1
    ORDER BY
      order_day
    "###);
}

#[test]
fn test_window_functions_03() {
    // lag must be recognized as window function, even outside of group context
    // rank must not have two OVER clauses
    let query = r###"
    from daily_orders
    derive [last_week = lag 7 num_orders]
    group month ( derive [total_month = sum num_orders])
    "###;

    assert_display_snapshot!((compile(query).unwrap()), @r###"
    SELECT
      *,
      LAG(num_orders, 7) OVER () AS last_week,
      SUM(num_orders) OVER (PARTITION BY month) AS total_month
    FROM
      daily_orders
    "###);
}

#[test]
fn test_window_functions_04() {
    // sort does not affects into groups, group undoes sorting
    let query = r###"
    from daily_orders
    sort day
    group month (derive [total_month = rank])
    derive [last_week = lag 7 num_orders]
    "###;

    assert_display_snapshot!((compile(query).unwrap()), @r###"
    SELECT
      *,
      RANK() OVER (PARTITION BY month) AS total_month,
      LAG(num_orders, 7) OVER () AS last_week
    FROM
      daily_orders
    "###);
}

#[test]
fn test_window_functions_05() {
    // sort does not leak out of groups
    let query = r###"
    from daily_orders
    sort day
    group month (sort num_orders | window expanding:true (derive rank))
    derive [num_orders_last_week = lag 7 num_orders]
    "###;
    assert_display_snapshot!((compile(query).unwrap()), @r###"
    SELECT
      *,
      RANK() OVER (
        PARTITION BY month
        ORDER BY
          num_orders ROWS BETWEEN UNBOUNDED PRECEDING
          AND CURRENT ROW
      ),
      LAG(num_orders, 7) OVER () AS num_orders_last_week
    FROM
      daily_orders
    "###);
}

#[test]
fn test_window_functions_06() {
    // detect sum as a window function, even without group or window
    assert_display_snapshot!((compile(r###"
    from foo
    derive [a = sum b]
    group c (
        derive [d = sum b]
    )
    "###).unwrap()), @r###"
    SELECT
      *,
      SUM(b) OVER () AS a,
      SUM(b) OVER (PARTITION BY c) AS d
    FROM
      foo
    "###);
}

#[test]
fn test_window_functions_07() {
    assert_display_snapshot!((compile(r###"
    from foo
    window expanding:true (
        derive [running_total = sum b]
    )
    "###).unwrap()), @r###"
    SELECT
      *,
      SUM(b) OVER (
        ROWS BETWEEN UNBOUNDED PRECEDING
        AND CURRENT ROW
      ) AS running_total
    FROM
      foo
    "###);
}

#[test]
fn test_window_functions_08() {
    assert_display_snapshot!((compile(r###"
    from foo
    window rolling:3 (
        derive [last_three = sum b]
    )
    "###).unwrap()), @r###"
    SELECT
      *,
      SUM(b) OVER (
        ROWS BETWEEN 2 PRECEDING
        AND CURRENT ROW
      ) AS last_three
    FROM
      foo
    "###);
}

#[test]
fn test_window_functions_09() {
    assert_display_snapshot!((compile(r###"
    from foo
    window rows:0..4 (
        derive [next_four_rows = sum b]
    )
    "###).unwrap()), @r###"
    SELECT
      *,
      SUM(b) OVER (
        ROWS BETWEEN CURRENT ROW
        AND 4 FOLLOWING
      ) AS next_four_rows
    FROM
      foo
    "###);
}

#[test]
fn test_window_functions_10() {
    assert_display_snapshot!((compile(r###"
    from foo
    sort day
    window range:-4..4 (
        derive [next_four_days = sum b]
    )
    "###).unwrap()), @r###"
    SELECT
      *,
      SUM(b) OVER (
        ORDER BY
          day RANGE BETWEEN 4 PRECEDING
          AND 4 FOLLOWING
      ) AS next_four_days
    FROM
      foo
    ORDER BY
      day
    "###);

    // TODO: add test for preceding
}

#[test]
fn test_name_resolving() {
    let query = r###"
    from numbers
    derive x = 5
    select [y = 6, z = x + y + a]
    "###;
    assert_display_snapshot!((compile(query).unwrap()), @r###"
    SELECT
      6 AS y,
      5 + 6 + a AS z
    FROM
      numbers
    "###);
}

#[test]
fn test_strings() {
    let query = r###"
    from empty_table_to_do
    select [
        x = "two households'",
        y = 'two households"',
        z = f"a {x} b' {y} c",
        v = f'a {x} b" {y} c',
    ]
    "###;
    assert_display_snapshot!((compile(query).unwrap()), @r###"
    SELECT
      'two households''' AS x,
      'two households"' AS y,
      CONCAT(
        'a ',
        'two households''',
        ' b'' ',
        'two households"',
        ' c'
      ) AS z,
      CONCAT(
        'a ',
        'two households''',
        ' b" ',
        'two households"',
        ' c'
      ) AS v
    FROM
      empty_table_to_do
    "###);
}

#[test]
fn test_filter() {
    // https://github.com/prql/prql/issues/469
    let query = r###"
    from employees
    filter [age > 25, age < 40]
    "###;

    assert!(compile(query).is_err());

    assert_display_snapshot!((compile(r###"
    from employees
    filter age > 25 and age < 40
    "###).unwrap()), @r###"
    SELECT
      *
    FROM
      employees
    WHERE
      age > 25
      AND age < 40
    "###);

    assert_display_snapshot!((compile(r###"
    from employees
    filter age > 25
    filter age < 40
    "###).unwrap()), @r###"
    SELECT
      *
    FROM
      employees
    WHERE
      age > 25
      AND age < 40
    "###);
}

#[test]
fn test_nulls() {
    assert_display_snapshot!((compile(r###"
    from employees
    select amount = null
    "###).unwrap()), @r###"
    SELECT
      NULL AS amount
    FROM
      employees
    "###);

    // coalesce
    assert_display_snapshot!((compile(r###"
    from employees
    derive amount = amount + 2 ?? 3 * 5
    "###).unwrap()), @r###"
    SELECT
      *,
      COALESCE(amount + 2, 3 * 5) AS amount
    FROM
      employees
    "###);

    // IS NULL
    assert_display_snapshot!((compile(r###"
    from employees
    filter first_name == null and null == last_name
    "###).unwrap()), @r###"
    SELECT
      *
    FROM
      employees
    WHERE
      first_name IS NULL
      AND last_name IS NULL
    "###);

    // IS NOT NULL
    assert_display_snapshot!((compile(r###"
    from employees
    filter first_name != null and null != last_name
    "###).unwrap()), @r###"
    SELECT
      *
    FROM
      employees
    WHERE
      first_name IS NOT NULL
      AND last_name IS NOT NULL
    "###);
}

#[test]
fn test_range() {
    assert_display_snapshot!((compile(r###"
    from employees
    take ..10
    "###).unwrap()), @r###"
    SELECT
      *
    FROM
      employees
    LIMIT
      10
    "###);

    assert_display_snapshot!((compile(r###"
    from employees
    take 5..10
    "###).unwrap()), @r###"
    SELECT
      *
    FROM
      employees
    LIMIT
      6 OFFSET 4
    "###);

    assert_display_snapshot!((compile(r###"
    from employees
    take 5..
    "###).unwrap()), @r###"
    SELECT
      *
    FROM
      employees OFFSET 4
    "###);

    assert_display_snapshot!((compile(r###"
    from employees
    take 5..5
    "###).unwrap()), @r###"
    SELECT
      *
    FROM
      employees
    LIMIT
      1 OFFSET 4
    "###);

    // should be one SELECT
    assert_display_snapshot!((compile(r###"
    from employees
    take 11..20
    take 1..5
    "###).unwrap()), @r###"
    SELECT
      *
    FROM
      employees
    LIMIT
      5 OFFSET 10
    "###);

    // should be two SELECTs
    assert_display_snapshot!((compile(r###"
    from employees
    take 11..20
    sort name
    take 1..5
    "###).unwrap()), @r###"
    WITH table_1 AS (
      SELECT
        *
      FROM
        employees
      LIMIT
        10 OFFSET 10
    )
    SELECT
      *
    FROM
      table_1
    ORDER BY
      name
    LIMIT
      5
    "###);
}

#[test]
fn test_distinct() {
    // window functions cannot materialize into where statement: CTE is needed
    assert_display_snapshot!((compile(r###"
    from employees
    derive rn = row_number
    filter rn > 2
    "###).unwrap()), @r###"
    WITH table_1 AS (
      SELECT
        *,
        ROW_NUMBER() OVER () AS rn
      FROM
        employees
    )
    SELECT
      *
    FROM
      table_1
    WHERE
      rn > 2
    "###);

    // basic distinct
    assert_display_snapshot!((compile(r###"
    from employees
    select first_name
    group first_name (take 1)
    "###).unwrap()), @r###"
    SELECT
      DISTINCT first_name
    FROM
      employees
    "###);

    // distinct on two columns
    assert_display_snapshot!((compile(r###"
    from employees
    select [first_name, last_name]
    group [first_name, last_name] (take 1)
    "###).unwrap()), @r###"
    SELECT
      DISTINCT first_name,
      last_name
    FROM
      employees
    "###);

    // TODO: this should not use DISTINCT but ROW_NUMBER and WHERE, because we want
    // row  distinct only over first_name and last_name.
    assert_display_snapshot!((compile(r###"
    from employees
    group [first_name, last_name] (take 1)
    "###).unwrap()), @r###"
    SELECT
      DISTINCT *
    FROM
      employees
    "###);

    // head
    assert_display_snapshot!((compile(r###"
    from employees
    group department (take 3)
    "###).unwrap()), @r###"
    WITH table_1 AS (
      SELECT
        *,
        ROW_NUMBER() OVER (PARTITION BY department) AS _expr_0
      FROM
        employees
    )
    SELECT
      *
    FROM
      table_1
    WHERE
      _expr_0 <= 3
    "###);

    assert_display_snapshot!((compile(r###"
    from employees
    group department (sort salary | take 2..3)
    "###).unwrap()), @r###"
    WITH table_1 AS (
      SELECT
        *,
        ROW_NUMBER() OVER (
          PARTITION BY department
          ORDER BY
            salary
        ) AS _expr_0
      FROM
        employees
    )
    SELECT
      *
    FROM
      table_1
    WHERE
      _expr_0 BETWEEN 2
      AND 3
    "###);
}

#[test]
fn test_dbt_query() {
    assert_display_snapshot!((compile(r###"
    from {{ ref('stg_orders') }}
    aggregate (min order_id)
    "###).unwrap()), @r###"
    SELECT
      MIN(order_id)
    FROM
      {{ ref('stg_orders') }}
    "###);
}

#[test]
fn test_join() {
    assert_display_snapshot!((compile(r###"
    from x
    join y [==id]
    "###).unwrap()), @r###"
    SELECT
      x.*,
      y.*
    FROM
      x
      JOIN y ON x.id = y.id
    "###);

    compile("from x | join y [==x.id]").unwrap_err();
}

#[test]
fn test_from_json() {
    // Test that the SQL generated from the JSON of the PRQL is the same as the raw PRQL
    let original_prql = r#"from e=employees
join salaries [==emp_no]
group [e.emp_no, e.gender] (
aggregate [
emp_salary = average salaries.salary
]
)
join de=dept_emp [==emp_no]
join dm=dept_manager [
(dm.dept_no == de.dept_no) and s"(de.from_date, de.to_date) OVERLAPS (dm.from_date, dm.to_date)"
]
group [dm.emp_no, gender] (
aggregate [
salary_avg = average emp_salary,
salary_sd = stddev emp_salary
]
)
derive mng_no = emp_no
join managers=employees [==emp_no]
derive mng_name = s"managers.first_name || ' ' || managers.last_name"
select [mng_name, managers.gender, salary_avg, salary_sd]"#;

    let sql_from_prql = parse(original_prql)
        .and_then(crate::semantic::resolve)
        .and_then(|rq| sql::compile(rq, None))
        .unwrap();

    let sql_from_json = crate::prql_to_pl(original_prql)
        .and_then(crate::json::from_pl)
        .and_then(|json| crate::json::to_pl(&json))
        .and_then(crate::pl_to_rq)
        .and_then(|rq| crate::rq_to_sql(rq, None))
        .unwrap();

    assert_eq!(sql_from_prql, sql_from_json);
}

#[test]
fn test_f_string() {
    let query = r###"
    from employees
    derive age = year_born - s'now()'
    select [
        f"Hello my name is {prefix}{first_name} {last_name}",
        f"and I am {age} years old."
    ]
    "###;

    let sql = compile(query).unwrap();
    assert_display_snapshot!(sql,
        @r###"
    SELECT
      CONCAT(
        'Hello my name is ',
        prefix,
        first_name,
        ' ',
        last_name
      ),
      CONCAT('and I am ', year_born - now(), ' years old.')
    FROM
      employees
    "###
    );
}

#[test]
fn test_sql_of_ast_1() {
    let query = r###"
    from employees
    filter country == "USA"
    group [title, country] (
        aggregate [average salary]
    )
    sort title
    take 20
    "###;

    let sql = compile(query).unwrap();
    assert_display_snapshot!(sql,
        @r###"
    SELECT
      title,
      country,
      AVG(salary)
    FROM
      employees
    WHERE
      country = 'USA'
    GROUP BY
      title,
      country
    ORDER BY
      title
    LIMIT
      20
    "###
    );
}

#[test]
fn test_sql_of_ast_2() {
    let query = r###"
    from employees
    aggregate sum_salary = s"sum({salary})"
    filter sum_salary > 100
    "###;
    let sql = compile(query).unwrap();
    assert_snapshot!(sql, @r###"
    SELECT
      sum(salary) AS sum_salary
    FROM
      employees
    HAVING
      sum(salary) > 100
    "###);
    assert!(sql.to_lowercase().contains(&"having".to_lowercase()));
}

#[test]
fn test_prql_to_sql_1() {
    let query = r#"
    from employees
    aggregate [
        count non_null:salary,
        sum salary,
    ]
    "#;
    let sql = compile(query).unwrap();
    assert_display_snapshot!(sql,
        @r###"
    SELECT
      COUNT(salary),
      SUM(salary)
    FROM
      employees
    "###
    )
}

#[test]
fn test_prql_to_sql_2() {
    let query = r#"
from employees
filter country == "USA"                           # Each line transforms the previous result.
derive [                                         # This adds columns / variables.
gross_salary = salary + payroll_tax,
gross_cost = gross_salary + benefits_cost      # Variables can use other variables.
]
filter gross_cost > 0
group [title, country] (
aggregate  [                                 # `by` are the columns to group by.
    average salary,                          # These are aggregation calcs run on each group.
    sum     salary,
    average gross_salary,
    sum     gross_salary,
    average gross_cost,
    sum_gross_cost = sum gross_cost,
    ct = count,
]
)
sort sum_gross_cost
filter ct > 200
take 20
"#;

    let sql = compile(query).unwrap();
    assert_display_snapshot!(sql)
}

#[test]
fn test_prql_to_sql_table() {
    // table
    let query = r#"
    table newest_employees = (
        from employees
        sort tenure
        take 50
    )
    table average_salaries = (
        from salaries
        group country (
            aggregate [
                average_country_salary = average salary
            ]
        )
    )
    from newest_employees
    join average_salaries [==country]
    select [name, salary, average_country_salary]
    "#;
    let sql = compile(query).unwrap();
    assert_display_snapshot!(sql,
        @r###"
    WITH average_salaries AS (
      SELECT
        country,
        AVG(salary) AS average_country_salary
      FROM
        salaries
      GROUP BY
        country
    ),
    newest_employees AS (
      SELECT
        *
      FROM
        employees
      ORDER BY
        tenure
      LIMIT
        50
    )
    SELECT
      newest_employees.name,
      newest_employees.salary,
      average_salaries.average_country_salary
    FROM
      newest_employees
      JOIN average_salaries ON newest_employees.country = average_salaries.country
    "###
    );
}

#[test]
fn test_nonatomic() {
    // A take, then two aggregates
    let query = r###"
        from employees
        take 20
        filter country == "USA"
        group [title, country] (
            aggregate [
                salary = average salary
            ]
        )
        group [title, country] (
            aggregate [
                sum_gross_cost = average salary
            ]
        )
        sort sum_gross_cost
    "###;

    assert_display_snapshot!((compile(query).unwrap()), @r###"
    WITH table_1 AS (
      SELECT
        title,
        country,
        salary
      FROM
        employees
      LIMIT
        20
    ), table_2 AS (
      SELECT
        title,
        country,
        AVG(salary) AS _expr_0
      FROM
        table_1
      WHERE
        country = 'USA'
      GROUP BY
        title,
        country
    )
    SELECT
      title,
      country,
      AVG(_expr_0) AS sum_gross_cost
    FROM
      table_2
    GROUP BY
      title,
      country
    ORDER BY
      sum_gross_cost
    "###);

    // A aggregate, then sort and filter
    let query = r###"
        from employees
        group [title, country] (
            aggregate [
                sum_gross_cost = average salary
            ]
        )
        sort sum_gross_cost
        filter sum_gross_cost > 0
    "###;

    assert_display_snapshot!((compile(query).unwrap()), @r###"
    SELECT
      title,
      country,
      AVG(salary) AS sum_gross_cost
    FROM
      employees
    GROUP BY
      title,
      country
    HAVING
      AVG(salary) > 0
    ORDER BY
      sum_gross_cost
    "###);
}

#[test]
/// Confirm a nonatomic table works.
fn test_nonatomic_table() {
    // A take, then two aggregates
    let query = r###"
    table a = (
        from employees
        take 50
        group country (aggregate [s"count(*)"])
    )
    from a
    join b [==country]
    select [name, salary, average_country_salary]
"###;

    assert_display_snapshot!((compile(query).unwrap()), @r###"
    WITH table_0 AS (
      SELECT
        country
      FROM
        employees
      LIMIT
        50
    ), a AS (
      SELECT
        country,
        count(*)
      FROM
        table_0
      GROUP BY
        country
    )
    SELECT
      b.name,
      b.salary,
      b.average_country_salary
    FROM
      a
      JOIN b ON a.country = b.country
    "###);
}

#[test]
fn test_table_names_between_splits() {
    let prql = r###"
    from employees
    join d=department [==dept_no]
    take 10
    derive emp_no = employees.emp_no
    join s=salaries [==emp_no]
    select [employees.emp_no, d.name, s.salary]
    "###;
    let result = compile(prql).unwrap();
    assert_display_snapshot!(result, @r###"
    WITH table_1 AS (
      SELECT
        employees.emp_no,
        d.name
      FROM
        employees
        JOIN department AS d ON employees.dept_no = d.dept_no
      LIMIT
        10
    )
    SELECT
      table_1.emp_no,
      table_1.name,
      s.salary
    FROM
      table_1
      JOIN salaries AS s ON table_1.emp_no = s.emp_no
    "###);

    let prql = r###"
    from e=employees
    take 10
    join salaries [==emp_no]
    select [e.*, salaries.salary]
    "###;
    let result = compile(prql).unwrap();
    assert_display_snapshot!(result, @r###"
    WITH table_1 AS (
      SELECT
        *
      FROM
        employees AS e
      LIMIT
        10
    )
    SELECT
      table_1.*,
      salaries.salary
    FROM
      table_1
      JOIN salaries ON table_1.emp_no = salaries.emp_no
    "###);
}

#[test]
fn test_table_alias() {
    // Alias on from
    let query = r###"
        from e = employees
        join salaries side:left [salaries.emp_no == e.emp_no]
        group [e.emp_no] (
            aggregate [
                emp_salary = average salaries.salary
            ]
        )
        select [emp_no, emp_salary]
    "###;

    assert_display_snapshot!((compile(query).unwrap()), @r###"
    SELECT
      e.emp_no,
      AVG(salaries.salary) AS emp_salary
    FROM
      employees AS e
      LEFT JOIN salaries ON salaries.emp_no = e.emp_no
    GROUP BY
      e.emp_no
    "###)
}

#[test]
fn test_dialects() {
    // Generic
    let query = r###"
    prql sql_dialect:generic
    from Employees
    select [FirstName, `last name`]
    take 3
    "###;

    assert_display_snapshot!((compile(query).unwrap()), @r###"
    SELECT
      "FirstName",
      "last name"
    FROM
      "Employees"
    LIMIT
      3
    "###);

    // SQL server
    let query = r###"
    prql sql_dialect:mssql
    from Employees
    select [FirstName, `last name`]
    take 3
    "###;

    assert_display_snapshot!((compile(query).unwrap()), @r###"
    SELECT
      TOP (3) "FirstName",
      "last name"
    FROM
      "Employees"
    "###);

    // MySQL
    let query = r###"
    prql sql_dialect:mysql
    from Employees
    select [FirstName, `last name`]
    take 3
    "###;

    assert_display_snapshot!((compile(query).unwrap()), @r###"
    SELECT
      `FirstName`,
      `last name`
    FROM
      `Employees`
    LIMIT
      3
    "###);
}

#[test]
fn test_dialect_clickhouse() {
    let query = r###"
    prql sql_dialect:clickhouse

    from github_json
    derive [event_type_dotted = `event.type`]
    "###;

    assert_display_snapshot!((compile(query).unwrap()), @r###"
    SELECT
      *,
      `event.type` AS event_type_dotted
    FROM
      github_json
    "###);
}

#[test]
fn test_ident_escaping() {
    // Generic
    let query = r###"
    from `anim"ls`
    derive [`čebela` = BeeName, medved = `bear's_name`]
    "###;

    assert_display_snapshot!((compile(query).unwrap()), @r###"
    SELECT
      *,
      "BeeName" AS "čebela",
      "bear's_name" AS medved
    FROM
      "anim""ls"
    "###);

    // MySQL
    let query = r###"
    prql sql_dialect:mysql

    from `anim"ls`
    derive [`čebela` = BeeName, medved = `bear's_name`]
    "###;

    assert_display_snapshot!((compile(query).unwrap()), @r###"
    SELECT
      *,
      `BeeName` AS `čebela`,
      `bear's_name` AS medved
    FROM
      `anim"ls`
    "###);
}

#[test]
fn test_literal() {
    let query = r###"
    from employees
    derive [always_true = true]
    "###;

    let sql = compile(query).unwrap();
    assert_display_snapshot!(sql,
        @r###"
    SELECT
      *,
      true AS always_true
    FROM
      employees
    "###
    );
}

#[test]
fn test_same_column_names() {
    // #820
    let query = r###"
table x = (
from x_table
select only_in_x = foo
)

table y = (
from y_table
select foo
)

from x
join y [foo == only_in_x]
"###;

    assert_display_snapshot!(compile(query).unwrap(),
        @r###"
    WITH x AS (
      SELECT
        foo AS only_in_x
      FROM
        x_table
    ),
    y AS (
      SELECT
        foo
      FROM
        y_table
    )
    SELECT
      x.only_in_x,
      y.foo
    FROM
      x
      JOIN y ON y.foo = x.only_in_x
    "###
    );
}

#[test]
fn test_double_aggregate() {
    // #941
    let query = r###"
    from numbers
    group [type] (
        aggregate [
            total_amt = sum amount,
        ]
        aggregate [
            max amount
        ]
    )
    "###;

    compile(query).unwrap_err();

    let query = r###"
    from numbers
    group [type] (
        aggregate [
            total_amt = sum amount,
            max amount
        ]
    )
    "###;

    assert_display_snapshot!(compile(query).unwrap(),
        @r###"
    SELECT
      type,
      SUM(amount) AS total_amt,
      MAX(amount)
    FROM
      numbers
    GROUP BY
      type
    "###
    );
}

#[test]
fn test_casting() {
    assert_display_snapshot!(compile(r###"
    from x
    select [a]
    derive [
        c = (a | as int) / 10
    ]
    "###).unwrap(),
        @r###"
    SELECT
      a,
      CAST(a AS int) / 10 AS c
    FROM
      x
    "###
    );
}

#[test]
/// Start testing some error messages. This can hopefully be expanded significantly.
fn test_errors() {
    assert_display_snapshot!(compile(r###"
    from x
    select a
    select b
    "###).unwrap_err(),
        @r###"
    Error:
    ╭─[:4:12]
    │
    4 │     select b
    ·            ┬
    ·            ╰── Unknown name b
    ───╯
    "###);
}

#[test]
fn test_toposort() {
    // #1183

    assert_display_snapshot!(compile(r###"
    table b = (
        from somesource
    )

    table a = (
        from b
    )

    from b
    "###).unwrap(),
        @r###"
    WITH b AS (
      SELECT
        *
      FROM
        somesource
    ),
    a AS (
      SELECT
        *
      FROM
        b
    )
    SELECT
      *
    FROM
      b
    "###
    );
}

#[test]
fn test_inline_tables() {
    assert_display_snapshot!(compile(r###"
    (
        from employees
        select [emp_id, name, surname, type, amount]
    )
    join s = (from salaries | select [emp_id, salary]) [==emp_id]
    "###).unwrap(),
        @r###"
    WITH table_1 AS (
      SELECT
        emp_id,
        salary
      FROM
        salaries
    )
    SELECT
      employees.emp_id,
      employees.name,
      employees.surname,
      employees.type,
      employees.amount,
      table_0.emp_id,
      table_0.salary
    FROM
      employees
      JOIN table_1 AS table_0 ON employees.emp_id = table_0.emp_id
    "###
    );
}

#[test]
fn test_filter_and_select_unchanged_alias() {
    // #1185

    assert_display_snapshot!(compile(r###"
    from account
    filter account.name != null
    select [name = account.name]
    "###).unwrap(),
        @r###"
    SELECT
      name
    FROM
      account
    WHERE
      name IS NOT NULL
    "###
    );
}

#[test]
fn test_filter_and_select_changed_alias() {
    // #1185
    assert_display_snapshot!(compile(r###"
    from account
    filter account.name != null
    select [renamed_name = account.name]
    "###).unwrap(),
        @r###"
    SELECT
      name AS renamed_name
    FROM
      account
    WHERE
      name IS NOT NULL
    "###
    );

    // #1207
    assert_display_snapshot!(compile(r###"
    from x
    filter name != "Bob"
    select name = name ?? "Default"
    "###).unwrap(),
        @r###"
    SELECT
      COALESCE(name, 'Default') AS name
    FROM
      x
    WHERE
      name <> 'Bob'
    "###
    );
}

#[test]
fn test_table_s_string() {
    assert_display_snapshot!(compile(r###"
    s"SELECT DISTINCT ON first_name, age FROM employees ORDER BY age ASC"
    "###).unwrap(),
        @r###"
    WITH table_1 AS (
      SELECT
        DISTINCT ON first_name,
        age
      FROM
        employees
      ORDER BY
        age ASC
    )
    SELECT
    FROM
      table_1 AS table_0
    "###
    );

    assert_display_snapshot!(compile(r###"
    from s"""
        SELECT DISTINCT ON first_name, id, age FROM employees ORDER BY age ASC
    """
    join s = s"SELECT * FROM salaries" [==id]
    "###).unwrap(),
        @r###"
    WITH table_2 AS (
      SELECT
        DISTINCT ON first_name,
        id,
        age
      FROM
        employees
      ORDER BY
        age ASC
    ),
    table_3 AS (
      SELECT
        *
      FROM
        salaries
    )
    SELECT
      table_0.*,
      table_1.*
    FROM
      table_2 AS table_0
      JOIN table_3 AS table_1 ON table_0.id = table_1.id
    "###
    );

    assert_display_snapshot!(compile(r###"
    from s"""SELECT * FROM employees"""
    filter country == "USA"
    "###).unwrap(),
        @r###"
    WITH table_1 AS (
      SELECT
        *
      FROM
        employees
    )
    SELECT
      *
    FROM
      table_1 AS table_0
    WHERE
      country = 'USA'
    "###
    );

    assert_display_snapshot!(compile(r###"
    from e=s"""SELECT * FROM employees"""
    filter e.country == "USA"
    "###).unwrap(),
        @r###"
    WITH table_1 AS (
      SELECT
        *
      FROM
        employees
    )
    SELECT
      *
    FROM
      table_1 AS table_0
    WHERE
      country = 'USA'
    "###
    );
}

#[test]
fn test_direct_table_references() {
    compile(
        r###"
    from x
    select s"{x}.field"
    "###,
    )
    .unwrap_err();

    compile(
        r###"
    from x
    select x
    "###,
    )
    .unwrap_err();
}

#[test]
fn test_name_shadowing() {
    assert_display_snapshot!(compile(
        r###"
    from x
    select [a, a, a = a + 1]
    "###).unwrap(),
        @r###"
    SELECT
      a AS _expr_0,
      a AS _expr_1,
      a + 1 AS a
    FROM
      x
    "###
    );

    assert_display_snapshot!(compile(
        r###"
    from x
    select a
    derive a
    derive a = a + 1
    derive a = a + 2
    "###).unwrap(),
        @r###"
    SELECT
      a AS _expr_0,
      a AS _expr_1,
      a + 1,
      a + 1 + 2 AS a
    FROM
      x
    "###
    );
}

#[test]
fn test_group_all() {
    assert_display_snapshot!(compile(
        r###"
    from e=employees
    take 10
    join salaries [==emp_no]
    group [e.*] (aggregate sal = (sum salaries.salary))
        "###).unwrap(),
        @r###"
    WITH table_1 AS (
      SELECT
        *
      FROM
        employees AS e
      LIMIT
        10
    )
    SELECT
      table_1.*,
      SUM(salaries.salary) AS sal
    FROM
      table_1
      JOIN salaries ON table_1.emp_no = salaries.emp_no
    GROUP BY
      table_1.*
    "###
    );
}

#[test]
fn test_output_column_deduplication() {
    // #1249
    assert_display_snapshot!(compile(
        r###"
    from report
    derive r = s"RANK() OVER ()"
    filter r == 1
        "###).unwrap(),
        @r###"
    WITH table_1 AS (
      SELECT
        *,
        RANK() OVER () AS r
      FROM
        report
    )
    SELECT
      *
    FROM
      table_1
    WHERE
      r = 1
    "###
    );
}

#[test]
fn test_switch() {
    assert_display_snapshot!(compile(
        r###"
    from employees
    derive display_name = switch [
        nickname != null -> nickname,
        true -> f'{first_name} {last_name}'
    ]
        "###).unwrap(),
        @r###"
    SELECT
      *,
      CASE
        WHEN nickname IS NOT NULL THEN nickname
        ELSE CONCAT(first_name, ' ', last_name)
      END AS display_name
    FROM
      employees
    "###
    );

    assert_display_snapshot!(compile(
        r###"
    from employees
    derive display_name = switch [
        nickname != null -> nickname,
        first_name != null -> f'{first_name} {last_name}'
    ]
        "###).unwrap(),
        @r###"
    SELECT
      *,
      CASE
        WHEN nickname IS NOT NULL THEN nickname
        WHEN first_name IS NOT NULL THEN CONCAT(first_name, ' ', last_name)
        ELSE NULL
      END AS display_name
    FROM
      employees
    "###
    );
}

#[test]
fn test_sql_options() {
    let sql = crate::compile("from x").unwrap();

    assert!(sql.contains('\n'));
    assert!(sql.contains("-- Generated by"));

    let opts = sql::Options::default().no_signature().no_format().some();
    let sql = crate::parser::parse("from  x")
        .and_then(crate::semantic::resolve)
        .and_then(|rq| sql::compile(rq, opts))
        .unwrap();

    assert!(!sql.contains('\n'));
    assert!(!sql.contains("-- Generated by"));
}