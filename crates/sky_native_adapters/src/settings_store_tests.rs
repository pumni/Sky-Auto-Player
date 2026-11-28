use super::*;

#[test]
fn persisted_config_coercions_keep_distinct_integer_and_text_rules() {
    let boolean = Value::Bool(true);
    assert_eq!(parse_legacy_integer_setting(&boolean), None);
    assert_eq!(parse_legacy_profile_number(&boolean), Some(1.0));
    assert_eq!(parse_legacy_integer_setting(&Value::from(2.9)), Some(2));
    assert_eq!(parse_legacy_integer_setting(&Value::from(" 60 ")), Some(60));
    assert_eq!(
        parse_json_number_or_decimal_string(&Value::from(" 1.25 ")),
        Some(1.25)
    );

    assert_eq!(format_legacy_config_value(&Value::Null), "None");
    assert_eq!(format_legacy_config_value(&boolean), "True");
    assert_eq!(
        format_legacy_config_value(&serde_json::json!(["a'b", true, null])),
        r#"['a\'b', True, None]"#
    );
}
