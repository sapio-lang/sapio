//! Targeted bounds on schema compilation and repeated reference expansion.
//!
//! These count schema visits, not validation of an arbitrarily large instance.
//! Productive recursive schemas remain supported; this is not a global CPU or
//! memory budget for the native validator.

use referencing::{DefaultRetriever, Draft, Registry, Resolver, Uri};
use serde_json::Value;
use std::sync::Arc;

// A tiny acyclic schema can otherwise cause millions of repeated validations.
const MAX_EXPANDED_NODES: usize = 65_536;
// Reference chains can be deep even when the JSON document itself is shallow.
const MAX_SCHEMA_DEPTH: usize = 128;

pub(super) fn check(schema: &Value) -> Result<(), String> {
    // Check the physical schema tree before registry preparation or recursive
    // validator compilation. Literal const/default/example values are not
    // schema positions and must not acquire schema keyword semantics.
    let mut remaining = MAX_EXPANDED_NODES;
    check_structure(schema, 1, &mut remaining)?;

    let resource = Draft::Draft7.create_resource_ref(schema);
    // Match jsonschema's default base selection, including Draft 7's rule that
    // an ID adjacent to a reference does not establish a resource.
    let base = resource.id().unwrap_or("json-schema:///");
    let registry = Registry::new()
        .draft(Draft::Draft7)
        .retriever(DefaultRetriever)
        .add(base, resource)
        .and_then(|builder| builder.prepare())
        .map_err(|error| format!("Invalid schema reference: {error}"))?;
    let base = referencing::uri::from_str(base)
        .map_err(|error| format!("Invalid schema identifier: {error}"))?;
    let resolver = registry.resolver(base);
    ExpandedWalk {
        remaining: MAX_EXPANDED_NODES,
        active: Vec::new(),
    }
    .visit(schema, &resolver, 1, 0)
}

fn count_visit(depth: usize, remaining: &mut usize) -> Result<(), String> {
    if depth > MAX_SCHEMA_DEPTH {
        return Err(format!(
            "Schema reference/applicator depth exceeds {MAX_SCHEMA_DEPTH}"
        ));
    }
    if *remaining == 0 {
        return Err(format!(
            "Schema expansion exceeds {MAX_EXPANDED_NODES} nodes"
        ));
    }
    *remaining -= 1;
    Ok(())
}

fn check_draft(schema: &Value) -> Result<(), String> {
    if !(schema.is_boolean() || schema.is_object()) {
        return Err("A schema must be an object or boolean".into());
    }
    if let Some(declaration) = schema.get("$schema") {
        if declaration.as_str().map(Draft::from_schema_uri) != Some(Draft::Draft7) {
            return Err("Only Draft 7 schema declarations are supported".into());
        }
    }
    Ok(())
}

fn check_structure(schema: &Value, depth: usize, remaining: &mut usize) -> Result<(), String> {
    count_visit(depth, remaining)?;
    check_draft(schema)?;
    schema_children(schema, true, |child, _| {
        check_structure(child, depth + 1, remaining)
    })
}

struct ActiveSchema {
    identity: *const Value,
    base: Arc<Uri<String>>,
    instance_depth: usize,
}

struct ExpandedWalk {
    remaining: usize,
    active: Vec<ActiveSchema>,
}

impl ExpandedWalk {
    fn visit(
        &mut self,
        schema: &Value,
        resolver: &Resolver<'_>,
        depth: usize,
        instance_depth: usize,
    ) -> Result<(), String> {
        count_visit(depth, &mut self.remaining)?;
        check_draft(schema)?;
        let base = resolver.base_uri();
        let identity = std::ptr::from_ref(schema);
        if let Some(previous) = self
            .active
            .iter()
            .find(|entry| entry.identity == identity && entry.base == base)
        {
            return if instance_depth > previous.instance_depth {
                // This edge validates a descendant instance. Expanding it
                // again here would reject ordinary recursive Rust types.
                Ok(())
            } else {
                Err("Schema reference cycle does not descend into an instance".into())
            };
        }
        self.active.push(ActiveSchema {
            identity,
            base,
            instance_depth,
        });
        let result = self.visit_children(schema, resolver, depth, instance_depth);
        self.active.pop();
        result
    }

    fn visit_children(
        &mut self,
        schema: &Value,
        resolver: &Resolver<'_>,
        depth: usize,
        instance_depth: usize,
    ) -> Result<(), String> {
        if let Some(reference) = schema.get("$ref") {
            let reference = reference
                .as_str()
                .ok_or_else(|| "A schema reference must be a string".to_owned())?;
            let (target, target_resolver, draft) = resolver
                .lookup(reference)
                .map_err(|error| format!("Invalid schema reference: {error}"))?
                .into_inner();
            if draft != Draft::Draft7 {
                return Err("Only Draft 7 referenced schemas are supported".into());
            }
            // lookup already enters the target's resource scope. Applying its
            // relative ID again would change the meaning of its references.
            return self.visit(target, &target_resolver, depth + 1, instance_depth);
        }
        // Draft 7 ignores validation keywords adjacent to $ref. Otherwise,
        // include every applied schema child, including property/item schemas: placing
        // an expensive reference graph there must not evade the expansion cap.
        schema_children(schema, false, |child, descends| {
            let child_resolver = resolver
                .in_subresource(Draft::Draft7.create_resource_ref(child))
                .map_err(|error| format!("Invalid schema identifier: {error}"))?;
            self.visit(
                child,
                &child_resolver,
                depth + 1,
                instance_depth + usize::from(descends),
            )
        })
    }
}

// Draft 7 schema positions, with instance descent distinguished from schema
// composition. Dependencies validate the same object; definitions declare
// schemas without advancing the instance. Property names are scalar values.
fn schema_children(
    schema: &Value,
    include_inactive: bool,
    mut visit: impl FnMut(&Value, bool) -> Result<(), String>,
) -> Result<(), String> {
    let Some(object) = schema.as_object() else {
        return Ok(());
    };
    for (keyword, value) in object {
        // These positions still contain schemas for structural validation,
        // but Draft 7 does not apply them without their activating keyword.
        if !include_inactive {
            match keyword.as_str() {
                "definitions" => continue,
                "if" if !object.contains_key("then") && !object.contains_key("else") => continue,
                "then" | "else" if !object.contains_key("if") => continue,
                "additionalItems" if !object.get("items").is_some_and(Value::is_array) => continue,
                _ => {}
            }
        }
        match keyword.as_str() {
            "additionalItems" | "additionalProperties" | "contains" | "propertyNames" => {
                visit(value, true)?;
            }
            "if" | "then" | "else" | "not" => visit(value, false)?,
            "allOf" | "anyOf" | "oneOf" => {
                if let Some(children) = value.as_array() {
                    for child in children {
                        visit(child, false)?;
                    }
                }
            }
            "definitions" | "properties" | "patternProperties" => {
                if let Some(children) = value.as_object() {
                    for child in children.values() {
                        visit(child, keyword != "definitions")?;
                    }
                }
            }
            "items" => {
                if let Some(children) = value.as_array() {
                    for child in children {
                        visit(child, true)?;
                    }
                } else {
                    visit(value, true)?;
                }
            }
            "dependencies" => {
                if let Some(children) = value.as_object() {
                    for child in children.values().filter(|child| !child.is_array()) {
                        visit(child, false)?;
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use schemars::JsonSchema;
    use serde_json::{json, Map};

    fn dag(depth: usize) -> Value {
        let mut definitions = Map::new();
        definitions.insert("d0".into(), json!({"not": {"const": "reject"}}));
        for n in 1..=depth {
            let reference = format!("#/definitions/d{}", n - 1);
            definitions.insert(
                format!("d{n}"),
                json!({"allOf": [{"$ref": reference}, {"$ref": reference}]}),
            );
        }
        json!({"definitions": definitions, "$ref": format!("#/definitions/d{depth}")})
    }

    #[test]
    fn rejects_measured_exponential_reference_graphs() {
        assert!(check(&dag(13)).is_ok());
        for depth in [16, 24] {
            assert!(check(&dag(depth)).unwrap_err().contains("expansion"));
        }
    }

    #[test]
    fn property_wrapping_does_not_hide_reference_expansion() {
        let mut inner = dag(24);
        let reference = inner.as_object_mut().unwrap().remove("$ref").unwrap();
        inner["$id"] = json!("https://example.test/nested.json");
        inner["allOf"] = json!([{"$ref": reference}]);
        let schema = json!({"properties": {"nested": inner}});
        assert!(check(&schema).unwrap_err().contains("expansion"));
    }

    #[test]
    fn unused_definitions_are_declarations_not_applications() {
        let schema = json!({
            "type": "integer",
            "definitions": {"unused": {"$ref": "#"}}
        });
        check(&schema).unwrap();
        let validator = jsonschema::options()
            .with_draft(Draft::Draft7)
            .offline()
            .build(&schema)
            .unwrap();
        assert!(validator.is_valid(&json!(1)));
        assert!(!validator.is_valid(&json!("wrong")));
    }

    #[test]
    fn inactive_applicators_do_not_expand_references() {
        for keyword in ["if", "then", "else", "additionalItems"] {
            let mut schema = dag(16);
            let reference = schema.as_object_mut().unwrap().remove("$ref").unwrap();
            schema["type"] = json!("integer");
            schema[keyword] = json!({"$ref": reference});
            check(&schema).unwrap();
            let validator = jsonschema::options()
                .with_draft(Draft::Draft7)
                .offline()
                .build(&schema)
                .unwrap();
            assert!(validator.is_valid(&json!(1)));
            assert!(!validator.is_valid(&json!("wrong")));

            match keyword {
                "if" => schema["then"] = json!(true),
                "then" | "else" => schema["if"] = json!(true),
                "additionalItems" => schema["items"] = json!([true]),
                _ => unreachable!(),
            }
            assert!(check(&schema).unwrap_err().contains("expansion"));
        }
    }

    #[test]
    fn rejects_cycles_without_instance_descent() {
        for schema in [
            json!({"$ref": "#"}),
            json!({"not": {"$ref": "#"}}),
            json!({"definitions": {
                "a": {"allOf": [{"$ref": "#/definitions/b"}]},
                "b": {"allOf": [{"$ref": "#/definitions/a"}]}
            }, "$ref": "#/definitions/a"}),
        ] {
            assert!(check(&schema).unwrap_err().contains("cycle"));
        }
    }

    #[test]
    fn preserves_productive_schemars_recursion() {
        #[derive(JsonSchema)]
        #[allow(dead_code)]
        struct Node {
            value: u32,
            children: Vec<Node>,
        }
        let schema = serde_json::to_value(
            schemars::generate::SchemaSettings::draft07()
                .into_generator()
                .into_root_schema_for::<Node>(),
        )
        .unwrap();
        check(&schema).unwrap();
        let validator = jsonschema::options()
            .with_draft(Draft::Draft7)
            .offline()
            .build(&schema)
            .unwrap();
        assert!(validator.is_valid(&json!({"value": 1, "children": [
            {"value": 2, "children": []}
        ]})));
        assert!(!validator.is_valid(&json!({"value": 1, "children": [
            {"value": "wrong", "children": []}
        ]})));
    }

    #[test]
    fn literal_values_do_not_become_schemas() {
        check(&json!({
            "const": {"$ref": "#%FF", "$schema": "not a draft"},
            "enum": [{"$ref": "#"}],
            "default": {"$ref": "file:///unavailable"},
            "examples": [{"$ref": "http://unavailable"}]
        }))
        .unwrap();
    }

    #[test]
    fn ignores_draft7_reference_siblings() {
        check(&json!({
            "$ref": "#/definitions/allowed",
            "not": {"$ref": "#"},
            "definitions": {"allowed": true}
        }))
        .unwrap();
    }

    #[test]
    fn resolves_ids_anchors_and_escaped_pointers() {
        for schema in [
            json!({
                "$id": "https://example.test/root.json",
                "allOf": [{"$ref": "inner.json"}],
                "definitions": {"inner": {
                    "$id": "inner.json",
                    "type": "object",
                    "properties": {"value": {"$ref": "#/definitions/number"}},
                    "definitions": {"number": {"type": "integer"}}
                }}
            }),
            json!({
                "allOf": [{"$ref": "#number"}],
                "definitions": {"number": {"$id": "#number", "type": "integer"}}
            }),
            json!({
                "$ref": "#/definitions/a~1b~0c",
                "definitions": {"a/b~c": {"type": "integer"}}
            }),
        ] {
            check(&schema).unwrap();
            jsonschema::options()
                .with_draft(Draft::Draft7)
                .offline()
                .build(&schema)
                .unwrap();
        }
    }

    #[test]
    fn malformed_or_unavailable_references_return_errors() {
        for reference in [
            "#%FF",
            "#%",
            "#/%FF",
            "#/missing",
            "http://127.0.0.1:1/schema",
            "file:///unavailable",
        ] {
            assert!(check(&json!({"$ref": reference})).is_err());
        }
    }

    #[test]
    fn checks_structural_depth_before_compilation() {
        let mut schema = json!(true);
        for _ in 1..MAX_SCHEMA_DEPTH {
            schema = json!({"allOf": [schema]});
        }
        check(&schema).unwrap();
        schema = json!({"allOf": [schema]});
        assert!(check(&schema).unwrap_err().contains("depth"));
    }

    #[test]
    fn checks_shallow_reference_chain_depth() {
        let mut definitions = Map::new();
        definitions.insert("d0".into(), json!(true));
        for n in 1..=MAX_SCHEMA_DEPTH {
            definitions.insert(
                format!("d{n}"),
                json!({"$ref": format!("#/definitions/d{}", n - 1)}),
            );
        }
        let schema = json!({"definitions": definitions, "$ref": format!("#/definitions/d{MAX_SCHEMA_DEPTH}")});
        assert!(check(&schema).unwrap_err().contains("depth"));
    }

    #[test]
    fn expansion_budget_has_an_exact_boundary() {
        let mut children = vec![json!(true); MAX_EXPANDED_NODES - 1];
        check(&json!({"allOf": children})).unwrap();
        children.push(json!(true));
        assert!(check(&json!({"allOf": children}))
            .unwrap_err()
            .contains("expansion"));
    }
}
