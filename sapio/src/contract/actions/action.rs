//! Typed contract actions and their request boundary.
//!
//! `#[sapio::contract]` preserves ordinary methods and registers only marked
//! actions and spending policies. Actions declare `committed` or `suggested`
//! explicitly. A suggested action receives no fabricated default request.
//! `defaults = Self::proposals` provides a separate default-proposal callback.
//!
//! Typed handles reject requests belonging to a different action:
//! ```compile_fail
//! use sapio::{Context, contract::CompilationError, template::Template};
//! struct Payment;
//! #[sapio::contract]
//! impl Payment {
//!     #[action(suggested)]
//!     fn pay(&self, _ctx: Context, amount: u64) -> Result<Template, CompilationError> {
//!         Err(CompilationError::OutOfFunds)
//!     }
//! }
//! fn incorrect(ctx: Context) {
//!     Payment::pay_action().invoke(&Payment, ctx, "a different request type");
//! }
//! ```
//! Commitment is never inferred from an action's Rust body:
//! ```compile_fail
//! struct Payment;
//! #[sapio::contract]
//! impl Payment {
//!     #[action]
//!     fn pay(&self, _ctx: sapio::Context) -> Vec<sapio::template::Template> { vec![] }
//! }
//! ```

use super::ConditionallyCompileIfList;
use super::{CompilationError, Context, GuardList, TxTmplIt};
use crate::contract::macros::ContinuationSchema;
use crate::template::Template;
use sapio_base::effects::{EditableMapEffectDB, EffectPath, MapEffectDB, PathFragment};
use sapio_base::serialization_helpers::SArc;
use sapio_base::simp::{ContinuationPointLT, SIMPAttachableAt};
use serde::{de::DeserializeOwned, Serialize};
use std::sync::Arc;

/// The compiler-owned relationship between an action and its templates.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum TemplateKind {
    /// Every returned transaction is committed by the configured covenant.
    Committed,
    /// Returned transactions are proposals under the action's fixed policy.
    Suggested,
}

/// Normalize the common single-template return without hiding multiple alternatives.
pub trait IntoTemplates {
    /// Convert this result into the compiler's fallible stream.
    fn into_templates(self) -> TxTmplIt;
}
impl IntoTemplates for Template {
    fn into_templates(self) -> TxTmplIt {
        Ok(Box::new(std::iter::once(Ok(self))))
    }
}
impl<E: Into<CompilationError>> IntoTemplates for Result<Template, E> {
    fn into_templates(self) -> TxTmplIt {
        self.map_err(Into::into)?.into_templates()
    }
}
impl IntoTemplates for Vec<Template> {
    fn into_templates(self) -> TxTmplIt {
        Ok(Box::new(self.into_iter().map(Ok)))
    }
}
impl<E: Into<CompilationError>> IntoTemplates for Result<Vec<Template>, E> {
    fn into_templates(self) -> TxTmplIt {
        self.map_err(Into::into)?.into_templates()
    }
}
impl IntoTemplates for TxTmplIt {
    fn into_templates(self) -> TxTmplIt {
        self
    }
}

/// Metadata attached to an action's public request API.
pub type ActionMetadata<C> =
    fn(
        &C,
        Context,
    ) -> Result<Vec<Box<dyn SIMPAttachableAt<ContinuationPointLT>>>, CompilationError>;
/// A registered action. `None` explicitly omits an optional interface implementation.
pub type ActionFactory<C> = fn() -> Option<Box<dyn ErasedAction<C>>>;

/// Compiler interface after erasing an individual action's request type.
///
/// Policy discovery never fabricates a request. Only an explicit default
/// proposal callback or a supplied JSON request invokes transaction generation.
pub trait ErasedAction<C> {
    /// Explicit proposals made without a user request.
    fn default_templates(&self, contract: &C, ctx: Context) -> TxTmplIt;
    /// Decode and invoke this action's request, without routing through other actions.
    fn call_json(&self, contract: &C, ctx: Context, request: serde_json::Value) -> TxTmplIt;
    /// Whether this action accepts serialized requests.
    fn web_api(&self) -> bool;
    /// Compile-time action presence constraints.
    fn get_conditional_compile_if(&self) -> ConditionallyCompileIfList<'_, C>;
    /// Fixed authorization policy declarations.
    fn get_guard(&self) -> GuardList<'_, C>;
    /// Stable action name, used in request paths.
    fn get_name(&self) -> &Arc<String>;
    /// Optional schema for this action's own request type.
    fn get_schema(&self) -> &ContinuationSchema;
    /// Transaction commitment semantics.
    fn template_kind(&self) -> TemplateKind;
    /// Metadata evaluated at the attachment context.
    fn gen_simps(
        &self,
        contract: &C,
        ctx: Context,
    ) -> Result<Vec<Box<dyn SIMPAttachableAt<ContinuationPointLT>>>, CompilationError>;
}

/// One typed action, usable directly in Rust or erased at the compiler boundary.
///
/// Calling `invoke` constructs proposals; authorization is enforced when the
/// contract is compiled and when its resulting spending policy is satisfied.
/// A proposal's Rust checks do not automatically become spending predicates.
pub struct Action<C, Request> {
    name: Arc<String>,
    kind: TemplateKind,
    callback: fn(&C, Context, Request) -> TxTmplIt,
    defaults: Option<fn(&C, Context) -> TxTmplIt>,
    guards: Vec<fn() -> Option<super::Guard<C>>>,
    conditions: Vec<fn() -> Option<super::ConditionallyCompileIf<C>>>,
    metadata: Option<ActionMetadata<C>>,
    schema: ContinuationSchema,
    decode: Option<fn(serde_json::Value) -> Result<Request, serde_json::Error>>,
}
impl<C, Request> Action<C, Request> {
    /// Stable action name used in request paths.
    pub fn name(&self) -> &str {
        self.name.as_str()
    }

    /// Whether candidates are covenant-committed or proposed under a fixed policy.
    pub fn kind(&self) -> TemplateKind {
        self.kind
    }

    /// Schema of this action's own request, when a JSON interface is enabled.
    pub fn schema(&self) -> Option<&serde_json::Value> {
        self.schema.as_deref()
    }

    /// Declare a request callback. No default proposals or JSON API are implied.
    pub fn new(
        name: &str,
        kind: TemplateKind,
        callback: fn(&C, Context, Request) -> TxTmplIt,
    ) -> Self {
        Self {
            name: Arc::new(name.into()),
            kind,
            callback,
            defaults: None,
            guards: vec![],
            conditions: vec![],
            metadata: None,
            schema: None,
            decode: None,
        }
    }
    /// Require the conjunction of these guards.
    pub fn with_guards(mut self, guards: GuardList<'_, C>) -> Self {
        self.guards = guards.to_vec();
        self
    }
    /// Preserve the full required/nullable/absent compile-time condition algebra.
    pub fn with_conditions(mut self, conditions: ConditionallyCompileIfList<'_, C>) -> Self {
        self.conditions = conditions.to_vec();
        self
    }
    /// Supply explicit default proposals separately from the request type.
    pub fn with_defaults(mut self, defaults: fn(&C, Context) -> TxTmplIt) -> Self {
        self.defaults = Some(defaults);
        self
    }
    /// Attach optional request metadata.
    pub fn with_metadata(mut self, metadata: Option<ActionMetadata<C>>) -> Self {
        self.metadata = metadata;
        self
    }
    /// Construct proposals with a statically checked request.
    pub fn invoke(&self, contract: &C, ctx: Context, request: Request) -> TxTmplIt {
        (self.callback)(contract, ctx, request)
    }
    /// Erase only this action's request type for compiler registration.
    pub fn erase(self) -> Box<dyn ErasedAction<C>>
    where
        C: 'static,
        Request: 'static,
    {
        Box::new(self)
    }
    /// Build the exact effects path for this action under a contract instance.
    pub fn request_path(
        &self,
        contract_path: &EffectPath,
    ) -> Result<Arc<EffectPath>, CompilationError> {
        let name = PathFragment::try_from(self.name.clone())?;
        if !matches!(name, PathFragment::Named(_)) {
            return Err(CompilationError::InvalidPathName);
        }
        let action = EffectPath::push(Some(Arc::new(contract_path.clone())), PathFragment::Action);
        let action = EffectPath::push(Some(action), name);
        Ok(EffectPath::push(
            Some(action),
            match self.kind {
                TemplateKind::Committed => PathFragment::Next,
                TemplateKind::Suggested => PathFragment::Suggested,
            },
        ))
    }
    /// Encode one typed request for compilation or the WASM boundary.
    ///
    /// The request is attached only to this action. A unit request is JSON null;
    /// an absent request is represented by no entry, never by a default value.
    pub fn request(
        &self,
        contract_path: &EffectPath,
        request: &Request,
    ) -> Result<MapEffectDB, CompilationError>
    where
        Request: Serialize,
    {
        if self.decode.is_none() {
            return Err(CompilationError::WebAPIDisabled);
        }
        let mut effects: EditableMapEffectDB = MapEffectDB::default().into();
        effects
            .effects
            .entry(SArc(self.request_path(contract_path)?))
            .or_default()
            .insert(
                SArc(Arc::new("request".into())),
                serde_json::to_value(request).map_err(CompilationError::SerializationError)?,
            );
        Ok(effects.into())
    }
    /// Encode an ordered set of candidate requests for this action.
    /// Each request has a distinct deterministic label; none replaces another.
    pub fn requests<'a>(
        &self,
        contract_path: &EffectPath,
        requests: impl IntoIterator<Item = &'a Request>,
    ) -> Result<MapEffectDB, CompilationError>
    where
        Request: Serialize + 'a,
    {
        if self.decode.is_none() {
            return Err(CompilationError::WebAPIDisabled);
        }
        let mut entries = std::collections::BTreeMap::new();
        for (index, request) in requests.into_iter().enumerate() {
            entries.insert(
                SArc(Arc::new(format!("request_{index:020}"))),
                serde_json::to_value(request).map_err(CompilationError::SerializationError)?,
            );
        }
        let mut effects: EditableMapEffectDB = MapEffectDB::default().into();
        if !entries.is_empty() {
            effects
                .effects
                .insert(SArc(self.request_path(contract_path)?), entries);
        }
        Ok(effects.into())
    }
}
impl<C, Request: DeserializeOwned + schemars::JsonSchema + 'static> Action<C, Request> {
    /// Expose this action's own request schema and JSON decoder.
    pub fn with_json(mut self) -> Self {
        self.schema = Some(crate::contract::macros::get_schema_for::<Request>());
        self.decode = Some(serde_json::from_value::<Request>);
        self
    }
}
impl<C, Request> ErasedAction<C> for Action<C, Request> {
    fn default_templates(&self, contract: &C, ctx: Context) -> TxTmplIt {
        match self.defaults {
            Some(f) => f(contract, ctx),
            None => crate::contract::empty(),
        }
    }
    fn call_json(&self, contract: &C, ctx: Context, request: serde_json::Value) -> TxTmplIt {
        let decode = self.decode.ok_or(CompilationError::WebAPIDisabled)?;
        (self.callback)(
            contract,
            ctx,
            decode(request).map_err(CompilationError::DeserializationError)?,
        )
    }
    fn web_api(&self) -> bool {
        self.decode.is_some()
    }
    fn get_conditional_compile_if(&self) -> ConditionallyCompileIfList<'_, C> {
        &self.conditions
    }
    fn get_guard(&self) -> GuardList<'_, C> {
        &self.guards
    }
    fn get_name(&self) -> &Arc<String> {
        &self.name
    }
    fn get_schema(&self) -> &ContinuationSchema {
        &self.schema
    }
    fn template_kind(&self) -> TemplateKind {
        self.kind
    }
    fn gen_simps(
        &self,
        contract: &C,
        ctx: Context,
    ) -> Result<Vec<Box<dyn SIMPAttachableAt<ContinuationPointLT>>>, CompilationError> {
        match self.metadata {
            Some(f) => f(contract, ctx),
            None => Ok(vec![]),
        }
    }
}
