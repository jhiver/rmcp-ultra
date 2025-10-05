use std::{borrow::Cow, collections::HashSet, sync::Arc};

use futures::{FutureExt, future::BoxFuture};
use schemars::JsonSchema;

use crate::{
    handler::server::tool::{
        CallToolHandler, DynCallToolHandler, ToolCallContext, schema_for_type,
    },
    model::{CallToolResult, Tool, ToolAnnotations},
};

/// Handler for dynamically registered tools.
///
/// Implement this trait to create runtime-registered tools that can access
/// the service state and handle parameters dynamically.
pub trait DynamicToolHandler<S>: Send + Sync + 'static {
    /// Execute the tool with given parameters.
    ///
    /// # Arguments
    /// * `service` - Reference to the service instance
    /// * `params` - JSON parameters matching tool's inputSchema
    ///
    /// # Returns
    /// CallToolResult or error
    fn call(
        &self,
        service: &S,
        params: Option<crate::model::JsonObject>,
    ) -> BoxFuture<'static, Result<crate::model::CallToolResult, crate::ErrorData>>;
}

pub struct ToolRoute<S> {
    #[allow(clippy::type_complexity)]
    pub call: Arc<DynCallToolHandler<S>>,
    pub attr: crate::model::Tool,
}

impl<S> std::fmt::Debug for ToolRoute<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolRoute")
            .field("name", &self.attr.name)
            .field("description", &self.attr.description)
            .field("input_schema", &self.attr.input_schema)
            .finish()
    }
}

impl<S> Clone for ToolRoute<S> {
    fn clone(&self) -> Self {
        Self {
            call: self.call.clone(),
            attr: self.attr.clone(),
        }
    }
}

impl<S: Send + Sync + 'static> ToolRoute<S> {
    pub fn new<C, A>(attr: impl Into<Tool>, call: C) -> Self
    where
        C: CallToolHandler<S, A> + Send + Sync + Clone + 'static,
    {
        Self {
            call: Arc::new(move |context: ToolCallContext<S>| {
                let call = call.clone();
                context.invoke(call).boxed()
            }),
            attr: attr.into(),
        }
    }
    pub fn new_dyn<C>(attr: impl Into<Tool>, call: C) -> Self
    where
        C: for<'a> Fn(
                ToolCallContext<'a, S>,
            ) -> BoxFuture<'a, Result<CallToolResult, crate::ErrorData>>
            + Send
            + Sync
            + 'static,
    {
        Self {
            call: Arc::new(call),
            attr: attr.into(),
        }
    }
    pub fn name(&self) -> &str {
        &self.attr.name
    }
}

pub trait IntoToolRoute<S, A> {
    fn into_tool_route(self) -> ToolRoute<S>;
}

impl<S, C, A, T> IntoToolRoute<S, A> for (T, C)
where
    S: Send + Sync + 'static,
    C: CallToolHandler<S, A> + Send + Sync + Clone + 'static,
    T: Into<Tool>,
{
    fn into_tool_route(self) -> ToolRoute<S> {
        ToolRoute::new(self.0.into(), self.1)
    }
}

impl<S> IntoToolRoute<S, ()> for ToolRoute<S>
where
    S: Send + Sync + 'static,
{
    fn into_tool_route(self) -> ToolRoute<S> {
        self
    }
}

pub struct ToolAttrGenerateFunctionAdapter;
impl<S, F> IntoToolRoute<S, ToolAttrGenerateFunctionAdapter> for F
where
    S: Send + Sync + 'static,
    F: Fn() -> ToolRoute<S>,
{
    fn into_tool_route(self) -> ToolRoute<S> {
        (self)()
    }
}

pub trait CallToolHandlerExt<S, A>: Sized
where
    Self: CallToolHandler<S, A> + Send + Sync + Clone + 'static,
{
    fn name(self, name: impl Into<Cow<'static, str>>) -> WithToolAttr<Self, S, A>;
}

impl<C, S, A> CallToolHandlerExt<S, A> for C
where
    C: CallToolHandler<S, A> + Send + Sync + Clone + 'static,
{
    fn name(self, name: impl Into<Cow<'static, str>>) -> WithToolAttr<Self, S, A> {
        WithToolAttr {
            attr: Tool::new(
                name.into(),
                "",
                schema_for_type::<crate::model::JsonObject>(),
            ),
            call: self,
            _marker: std::marker::PhantomData,
        }
    }
}

pub struct WithToolAttr<C, S, A>
where
    C: CallToolHandler<S, A> + Send + Sync + Clone + 'static,
{
    pub attr: crate::model::Tool,
    pub call: C,
    pub _marker: std::marker::PhantomData<fn(S, A)>,
}

impl<C, S, A> IntoToolRoute<S, A> for WithToolAttr<C, S, A>
where
    C: CallToolHandler<S, A> + Send + Sync + Clone + 'static,
    S: Send + Sync + 'static,
{
    fn into_tool_route(self) -> ToolRoute<S> {
        ToolRoute::new(self.attr, self.call)
    }
}

impl<C, S, A> WithToolAttr<C, S, A>
where
    C: CallToolHandler<S, A> + Send + Sync + Clone + 'static,
{
    pub fn description(mut self, description: impl Into<Cow<'static, str>>) -> Self {
        self.attr.description = Some(description.into());
        self
    }
    pub fn parameters<T: JsonSchema>(mut self) -> Self {
        self.attr.input_schema = schema_for_type::<T>().into();
        self
    }
    pub fn parameters_value(mut self, schema: serde_json::Value) -> Self {
        self.attr.input_schema = crate::model::object(schema).into();
        self
    }
    pub fn annotation(mut self, annotation: impl Into<ToolAnnotations>) -> Self {
        self.attr.annotations = Some(annotation.into());
        self
    }
}
#[derive(Debug)]
pub struct ToolRouter<S> {
    #[allow(clippy::type_complexity)]
    pub map: std::collections::HashMap<Cow<'static, str>, ToolRoute<S>>,

    pub transparent_when_not_found: bool,

    // Track which tools were registered dynamically
    dynamic_tool_names: HashSet<String>,
}

impl<S> Default for ToolRouter<S> {
    fn default() -> Self {
        Self {
            map: std::collections::HashMap::new(),
            transparent_when_not_found: false,
            dynamic_tool_names: HashSet::new(),
        }
    }
}
impl<S> Clone for ToolRouter<S> {
    fn clone(&self) -> Self {
        Self {
            map: self.map.clone(),
            transparent_when_not_found: self.transparent_when_not_found,
            dynamic_tool_names: self.dynamic_tool_names.clone(),
        }
    }
}

impl<S> IntoIterator for ToolRouter<S> {
    type Item = ToolRoute<S>;
    type IntoIter = std::collections::hash_map::IntoValues<Cow<'static, str>, ToolRoute<S>>;

    fn into_iter(self) -> Self::IntoIter {
        self.map.into_values()
    }
}

impl<S> ToolRouter<S>
where
    S: Send + Sync + 'static,
{
    pub fn new() -> Self {
        Self {
            map: std::collections::HashMap::new(),
            transparent_when_not_found: false,
            dynamic_tool_names: HashSet::new(),
        }
    }
    pub fn with_route<R, A>(mut self, route: R) -> Self
    where
        R: IntoToolRoute<S, A>,
    {
        self.add_route(route.into_tool_route());
        self
    }

    pub fn add_route(&mut self, item: ToolRoute<S>) {
        self.map.insert(item.attr.name.clone(), item);
    }

    pub fn merge(&mut self, other: ToolRouter<S>) {
        for item in other.map.into_values() {
            self.add_route(item);
        }
    }

    pub fn remove_route(&mut self, name: &str) {
        self.map.remove(name);
    }
    pub fn has_route(&self, name: &str) -> bool {
        self.map.contains_key(name)
    }
    pub async fn call(
        &self,
        context: ToolCallContext<'_, S>,
    ) -> Result<CallToolResult, crate::ErrorData> {
        let item = self
            .map
            .get(context.name())
            .ok_or_else(|| crate::ErrorData::invalid_params("tool not found", None))?;

        let result = (item.call)(context).await?;

        Ok(result)
    }

    pub fn list_all(&self) -> Vec<crate::model::Tool> {
        self.map.values().map(|item| item.attr.clone()).collect()
    }

    /// Register a tool at runtime
    ///
    /// # Arguments
    /// * `name` - Unique tool name
    /// * `description` - Optional description
    /// * `input_schema` - JSON Schema for parameters
    /// * `handler` - Dynamic tool handler implementation
    pub fn register_dynamic_tool(
        &mut self,
        name: String,
        description: Option<String>,
        input_schema: serde_json::Value,
        handler: Arc<dyn DynamicToolHandler<S>>,
    ) -> Result<(), crate::model::ToolRegistrationError> {
        use crate::model::ToolRegistrationError;

        // Validate name
        if name.is_empty() {
            return Err(ToolRegistrationError::InvalidName(
                "Name cannot be empty".to_string(),
            ));
        }

        // Check duplicates
        if self.map.contains_key(name.as_str()) {
            return Err(ToolRegistrationError::DuplicateTool(name));
        }

        // Validate schema is object
        let schema_obj = input_schema.as_object().ok_or_else(|| {
            ToolRegistrationError::InvalidSchema("Schema must be an object".to_string())
        })?;

        // Create tool definition
        let tool = if let Some(desc) = description {
            Tool::new(Cow::Owned(name.clone()), desc, schema_obj.clone())
        } else {
            Tool::new(Cow::Owned(name.clone()), "", schema_obj.clone())
        };

        // Create route with dynamic handler wrapper
        let route = ToolRoute::new_dyn(tool, move |context| {
            let handler = Arc::clone(&handler);
            Box::pin(async move { handler.call(context.service, context.arguments).await })
        });

        // Add to router and track as dynamic
        self.dynamic_tool_names.insert(name.clone());
        self.add_route(route);

        Ok(())
    }

    /// Remove a dynamically registered tool
    ///
    /// Only dynamic tools can be unregistered. Static tools (from macros) cannot be removed.
    pub fn unregister_tool(&mut self, name: &str) -> Result<(), crate::model::ToolNotFoundError> {
        if !self.dynamic_tool_names.contains(name) {
            return Err(crate::model::ToolNotFoundError::NotFound(name.to_string()));
        }

        self.dynamic_tool_names.remove(name);
        self.remove_route(name);
        Ok(())
    }

    /// Check if a tool exists
    pub fn has_tool(&self, name: &str) -> bool {
        self.has_route(name)
    }

    /// Get all tool names
    pub fn tool_names(&self) -> Vec<String> {
        self.map.keys().map(|k| k.to_string()).collect()
    }

    /// Count of dynamically registered tools
    pub fn dynamic_tool_count(&self) -> usize {
        self.dynamic_tool_names.len()
    }

    /// Count of statically registered tools (from macros)
    pub fn static_tool_count(&self) -> usize {
        self.map.len() - self.dynamic_tool_names.len()
    }
}

impl<S> std::ops::Add<ToolRouter<S>> for ToolRouter<S>
where
    S: Send + Sync + 'static,
{
    type Output = Self;

    fn add(mut self, other: ToolRouter<S>) -> Self::Output {
        self.merge(other);
        self
    }
}

impl<S> std::ops::AddAssign<ToolRouter<S>> for ToolRouter<S>
where
    S: Send + Sync + 'static,
{
    fn add_assign(&mut self, other: ToolRouter<S>) {
        self.merge(other);
    }
}
