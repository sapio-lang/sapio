//! Exact callable interface matching for locally referenced Draft 7 schemas.
//!
//! Documentation and definition names do not affect an interface. Validation
//! constraints and semantic identities do. This deliberately does not attempt
//! schema subtyping or prove that a module implements its declared semantics.

use serde_json::Value;
use std::collections::BTreeSet;

/// Compare two independently generated schema roots. Local references and
/// productive recursive types are supported; external references fail closed.
/// Callers must separately validate both schemas before using their values.
pub fn schemas_match(expected: &Value, actual: &Value) -> bool {
    Matcher {
        left: expected,
        right: actual,
        seen: BTreeSet::new(),
        remaining: 65_536,
    }
    .schema(expected, actual, 0)
}

struct Matcher<'a> {
    left: &'a Value,
    right: &'a Value,
    seen: BTreeSet<(usize, usize)>,
    remaining: usize,
}

fn dereference<'a>(
    mut value: &'a Value,
    root: &'a Value,
) -> Option<(&'a Value, Option<&'a Value>)> {
    let mut semantic = None;
    for _ in 0..128 {
        semantic = semantic.or_else(|| value.get("x-sapio-type"));
        let Some(reference) = value.get("$ref") else {
            return Some((value, semantic));
        };
        let reference = reference.as_str()?.strip_prefix('#')?;
        value = root.pointer(reference)?;
    }
    None
}

fn documentation(key: &str) -> bool {
    matches!(
        key,
        "$schema"
            | "$id"
            | "title"
            | "description"
            | "$comment"
            | "default"
            | "examples"
            | "definitions"
            | "$defs"
    )
}

impl Matcher<'_> {
    fn schema(&mut self, left: &Value, right: &Value, depth: usize) -> bool {
        if self.remaining == 0 || depth > 128 {
            return false;
        }
        self.remaining -= 1;
        let (Some((left, left_type)), Some((right, right_type))) =
            (dereference(left, self.left), dereference(right, self.right))
        else {
            return false;
        };
        if left_type != right_type {
            return false;
        }
        // Nested resource IDs would change the reference base. Rust-generated
        // signatures use root-local references; unsupported scopes fail closed.
        if (left.get("$id").is_some() && !std::ptr::eq(left, self.left))
            || (right.get("$id").is_some() && !std::ptr::eq(right, self.right))
        {
            return false;
        }
        if !self.seen.insert((
            std::ptr::from_ref(left) as usize,
            std::ptr::from_ref(right) as usize,
        )) {
            return true;
        }
        match (left.as_object(), right.as_object()) {
            (Some(left), Some(right)) => {
                let left_keys: BTreeSet<_> = left
                    .keys()
                    .filter(|key| !documentation(key) && key.as_str() != "x-sapio-type")
                    .collect();
                let right_keys: BTreeSet<_> = right
                    .keys()
                    .filter(|key| !documentation(key) && key.as_str() != "x-sapio-type")
                    .collect();
                left_keys == right_keys
                    && left_keys.into_iter().all(|key| {
                        let l = &left[key];
                        let r = &right[key];
                        match key.as_str() {
                            "properties" | "patternProperties" | "dependencies" => {
                                self.map(l, r, depth + 1)
                            }
                            "items"
                            | "additionalItems"
                            | "additionalProperties"
                            | "contains"
                            | "propertyNames"
                            | "not"
                            | "if"
                            | "then"
                            | "else" => self.child(l, r, depth + 1),
                            "allOf" | "anyOf" | "oneOf" => self.sequence(l, r, depth + 1),
                            "x-sapio-module" => {
                                l.as_object().is_some_and(|map| map.len() == 2)
                                    && r.as_object().is_some_and(|map| map.len() == 2)
                                    && ["arguments", "returns"].iter().all(|side| {
                                        match (l.get(side), r.get(side)) {
                                            (Some(l), Some(r)) => schemas_match(l, r),
                                            _ => false,
                                        }
                                    })
                            }
                            "required" => match (l.as_array(), r.as_array()) {
                                (Some(l), Some(r)) => {
                                    l.len() == r.len() && l.iter().all(|item| r.contains(item))
                                }
                                _ => l == r,
                            },
                            _ => l == r,
                        }
                    })
            }
            _ => left == right,
        }
    }

    fn map(&mut self, left: &Value, right: &Value, depth: usize) -> bool {
        match (left.as_object(), right.as_object()) {
            (Some(left), Some(right)) => {
                left.len() == right.len()
                    && left.iter().all(|(key, value)| {
                        right
                            .get(key)
                            .is_some_and(|other| self.child(value, other, depth))
                    })
            }
            _ => false,
        }
    }

    fn child(&mut self, left: &Value, right: &Value, depth: usize) -> bool {
        if left.is_array() || right.is_array() {
            self.sequence(left, right, depth)
        } else {
            self.schema(left, right, depth)
        }
    }

    fn sequence(&mut self, left: &Value, right: &Value, depth: usize) -> bool {
        match (left.as_array(), right.as_array()) {
            (Some(left), Some(right)) => {
                left.len() == right.len()
                    && left
                        .iter()
                        .zip(right)
                        .all(|(l, r)| self.schema(l, r, depth))
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn resolved_types_ignore_documentation_but_retain_semantic_identity() {
        let expected = json!({"title":"Caller", "$ref":"#/definitions/A", "definitions":{"A":{"type":"integer", "x-sapio-type":"bitcoin.satoshis", "minimum":0}}});
        let actual = json!({"title":"Producer", "type":"integer", "x-sapio-type":"bitcoin.satoshis", "minimum":0});
        assert!(schemas_match(&expected, &actual));
        let mut annotated_reference = expected.clone();
        annotated_reference["x-sapio-type"] = "bitcoin.relative-blocks".into();
        assert!(!schemas_match(&annotated_reference, &actual));
        for changed in [
            json!({"type":"integer", "x-sapio-type":"bitcoin.relative-blocks", "minimum":0}),
            json!({"type":"integer", "minimum":0}),
            json!({"type":"integer", "x-sapio-type":"bitcoin.satoshis", "minimum":1}),
        ] {
            assert!(!schemas_match(&expected, &changed));
        }
        assert!(!schemas_match(
            &json!({"const":{"title":"a"}}),
            &json!({"const":{"title":"b"}})
        ));
        assert!(!schemas_match(
            &json!({"$ref":"https://example.invalid/schema"}),
            &actual
        ));
    }

    #[test]
    fn recursive_types_match_after_renaming_definitions() {
        let left = json!({"$ref":"#/definitions/A", "definitions":{"A":{"type":"object", "properties":{"next":{"$ref":"#/definitions/A"},"key":{"type":"string"}}}}});
        let mut right: Value = serde_json::from_str(
            &left
                .to_string()
                .replace("definitions/A", "definitions/B")
                .replace("\"A\":", "\"B\":"),
        )
        .unwrap();
        assert!(schemas_match(&left, &right));
        right["definitions"]["B"]["properties"]["key"]["type"] = "integer".into();
        assert!(!schemas_match(&left, &right));
    }
}
