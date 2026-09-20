use serde_json::Value;

use crate::manifest::Precedence;

pub fn merge_values(declared: &Value, existing: &Value, precedence: Precedence) -> Value {
    match (declared, existing) {
        (Value::Object(declared_map), Value::Object(existing_map)) => {
            let mut result = declared_map.clone();

            for (key, existing_value) in existing_map {
                match result.get(key) {
                    Some(declared_value) => {
                        let merged = merge_values(declared_value, existing_value, precedence);

                        result.insert(key.clone(), merged);
                    }

                    None => {
                        result.insert(key.clone(), existing_value.clone());
                    }
                }
            }

            Value::Object(result)
        }

        _ => match precedence {
            Precedence::Existing => existing.clone(),
            Precedence::Declared => declared.clone(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn existing_scalar_wins() {
        let declared = json!({
            "theme": "dark",
            "fontSize": 12
        });

        let existing = json!({
            "fontSize": 18
        });

        let result = merge_values(&declared, &existing, Precedence::Existing);

        assert_eq!(
            result,
            json!({
                "theme": "dark",
                "fontSize": 18
            })
        );
    }

    #[test]
    fn declared_scalar_wins() {
        let declared = json!({
            "theme": "dark",
            "fontSize": 12
        });

        let existing = json!({
            "fontSize": 18
        });

        let result = merge_values(&declared, &existing, Precedence::Declared);

        assert_eq!(
            result,
            json!({
                "theme": "dark",
                "fontSize": 12
            })
        );
    }

    #[test]
    fn objects_merge_recursively() {
        let declared = json!({
            "editor": {
                "theme": "dark",
                "fontSize": 12
            }
        });

        let existing = json!({
            "editor": {
                "fontSize": 18
            }
        });

        let result = merge_values(&declared, &existing, Precedence::Existing);

        assert_eq!(
            result,
            json!({
                "editor": {
                    "theme": "dark",
                    "fontSize": 18
                }
            })
        );
    }

    #[test]
    fn arrays_are_replaced_as_units() {
        let declared = json!({
            "plugins": ["a", "b"]
        });

        let existing = json!({
            "plugins": ["c"]
        });

        let result = merge_values(&declared, &existing, Precedence::Existing);

        assert_eq!(
            result,
            json!({
                "plugins": ["c"]
            })
        );
    }
}
