// prqlc/tests/codegen_tests.rs

use prql_compiler::compile;
use prql_compiler::Options;

#[test]
fn test_codegen_for_shortest_track_by_artist() {
    let prql_query = r#"
        from tracks
        filter artist == "jeff"
        aggregate {
            shortest = min length,
        }
    "#;

    let expected_sql = r#"
        SELECT
          MIN(length) AS shortest
        FROM
          tracks
        WHERE
          artist = 'jeff'
    "#.trim();

    let options = Options::default();
    let compiled_sql = compile(prql_query, &options).expect("Failed to compile PRQL query");

    assert_eq!(compiled_sql.trim(), expected_sql);
}

