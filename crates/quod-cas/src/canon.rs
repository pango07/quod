use serde::Serialize;
use serde_json::Value;

#[derive(Debug, thiserror::Error)]
pub enum CanonError {
    #[error("serialize: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("canonical JSON allows integers only, got {0}")]
    NonInteger(serde_json::Number),
}

/// Canonical JSON: keys sorted bytewise, no whitespace, UTF-8, integers only.
///
/// Keys are sorted here explicitly rather than relying on serde_json's map
/// type, which changes order if any crate in the build enables
/// `preserve_order`.
pub fn canonical_json<T: Serialize>(value: &T) -> Result<String, CanonError> {
    let v = serde_json::to_value(value)?;
    let mut out = String::new();
    write_value(&v, &mut out)?;
    Ok(out)
}

fn write_value(v: &Value, out: &mut String) -> Result<(), CanonError> {
    match v {
        Value::Null | Value::Bool(_) | Value::String(_) => out.push_str(&v.to_string()),
        Value::Number(n) => {
            if !(n.is_i64() || n.is_u64()) {
                return Err(CanonError::NonInteger(n.clone()));
            }
            out.push_str(&n.to_string());
        }
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_value(item, out)?;
            }
            out.push(']');
        }
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
            out.push('{');
            for (i, k) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String(k.clone()).to_string());
                out.push(':');
                write_value(&map[k], out)?;
            }
            out.push('}');
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use serde_json::json;

    #[test]
    fn sorts_keys_and_strips_whitespace() {
        let v = json!({"b": 1, "a": [true, null, "x y"], "B": {"z": 0, "y": -2}});
        assert_eq!(
            canonical_json(&v).unwrap(),
            r#"{"B":{"y":-2,"z":0},"a":[true,null,"x y"],"b":1}"#
        );
    }

    #[test]
    fn rejects_floats() {
        assert!(canonical_json(&json!({"x": 1.5})).is_err());
    }

    fn arb_json() -> impl Strategy<Value = Value> {
        let leaf = prop_oneof![
            Just(Value::Null),
            any::<bool>().prop_map(Value::Bool),
            any::<i64>().prop_map(|n| json!(n)),
            ".*".prop_map(Value::String),
        ];
        leaf.prop_recursive(4, 32, 6, |inner| {
            prop_oneof![
                prop::collection::vec(inner.clone(), 0..6).prop_map(Value::Array),
                prop::collection::btree_map(".*", inner, 0..6)
                    .prop_map(|m| Value::Object(m.into_iter().collect())),
            ]
        })
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]
        #[test]
        fn idempotent(v in arb_json()) {
            let once = canonical_json(&v).unwrap();
            let reparsed: Value = serde_json::from_str(&once).unwrap();
            prop_assert_eq!(canonical_json(&reparsed).unwrap(), once);
        }
    }
}
