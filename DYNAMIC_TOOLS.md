# Dynamic Tool Registration

**Version:** 0.8.0-dynamic.1
**Status:** Implemented and Tested

## Overview

This document describes the dynamic tool registration feature added to rmcp-ultra, a fork of the official rmcp (Model Context Protocol Rust SDK). This feature enables **runtime tool registration** from databases or other dynamic sources, complementing the existing compile-time macro-based tool registration.

### Why This Feature?

The original rmcp library only supports **compile-time** tool registration using procedural macros (`#[tool]`, `#[tool_router]`). This works well for static tools known at compile time, but doesn't support use cases where:

- Tools are stored in a database
- Tools need to be added/removed at runtime
- Each tool instance should appear as a separate tool in MCP's `tools/list`
- Tool definitions come from external configuration

**Example Use Case:** [SaraMCP](https://github.com/jhiver/saramcp) loads HTTP API tool instances from SQLite, where each instance has different parameters and configurations. Each instance needs to appear as a separate tool to MCP clients.

## What's New

### 1. DynamicToolHandler Trait

A new trait for implementing runtime-registered tools:

```rust
pub trait DynamicToolHandler<S>: Send + Sync + 'static {
    fn call(
        &self,
        service: &S,
        params: Option<JsonObject>,
    ) -> BoxFuture<'static, Result<CallToolResult, ErrorData>>;
}
```

- **Generic over service type** - Access your server's state
- **Async-friendly** - Returns `BoxFuture` for async operations
- **Type-safe** - Compile-time checks with Rust's type system

### 2. New ToolRouter Methods

Extended the existing `ToolRouter<S>` with these methods:

#### `register_dynamic_tool()`

Register a tool at runtime:

```rust
pub fn register_dynamic_tool(
    &mut self,
    name: String,
    description: Option<String>,
    input_schema: serde_json::Value,
    handler: Arc<dyn DynamicToolHandler<S>>,
) -> Result<(), ToolRegistrationError>
```

**Validations:**
- ✅ Name must not be empty
- ✅ Name must be unique (no duplicates with static or dynamic tools)
- ✅ Input schema must be a JSON object

#### `unregister_tool()`

Remove a dynamically registered tool:

```rust
pub fn unregister_tool(
    &mut self,
    name: &str
) -> Result<(), ToolNotFoundError>
```

**Safety:** Only dynamic tools can be unregistered. Static tools (from macros) are protected.

#### Helper Methods

```rust
pub fn has_tool(&self, name: &str) -> bool;
pub fn tool_names(&self) -> Vec<String>;
pub fn dynamic_tool_count(&self) -> usize;
pub fn static_tool_count(&self) -> usize;
```

### 3. Error Types

Two new error types using `thiserror`:

```rust
#[derive(Debug, Error, Clone)]
pub enum ToolRegistrationError {
    #[error("Tool '{0}' already registered")]
    DuplicateTool(String),

    #[error("Invalid tool name: {0}")]
    InvalidName(String),

    #[error("Invalid input schema: {0}")]
    InvalidSchema(String),
}

#[derive(Debug, Error, Clone)]
pub enum ToolNotFoundError {
    #[error("Tool '{0}' not found")]
    NotFound(String),
}
```

## Usage Examples

### Basic Example: Echo Tool

```rust
use rmcp::handler::server::router::tool::{DynamicToolHandler, ToolRouter};
use rmcp::model::{CallToolResult, Content, JsonObject};
use futures::future::BoxFuture;
use serde_json::json;
use std::sync::Arc;

// 1. Define your handler
struct EchoHandler;

impl<S> DynamicToolHandler<S> for EchoHandler {
    fn call(
        &self,
        _service: &S,
        params: Option<JsonObject>,
    ) -> BoxFuture<'static, Result<CallToolResult, rmcp::ErrorData>> {
        Box::pin(async move {
            let message = params
                .as_ref()
                .and_then(|p| p.get("message"))
                .and_then(|v| v.as_str())
                .unwrap_or("default");

            Ok(CallToolResult::success(vec![Content::text(message)]))
        })
    }
}

// 2. Register the tool
let mut router = ToolRouter::new();

let schema = json!({
    "type": "object",
    "properties": {
        "message": {"type": "string"}
    },
    "required": ["message"]
});

router.register_dynamic_tool(
    "echo".to_string(),
    Some("Echo a message back".to_string()),
    schema,
    Arc::new(EchoHandler),
)?;

// 3. Use in your server
// The router will now dispatch "echo" tool calls to EchoHandler
```

### Advanced Example: Database-Driven Tools

```rust
use sqlx::SqlitePool;
use std::sync::Arc;

#[derive(Clone)]
pub struct MyServer {
    pool: SqlitePool,
    tool_router: ToolRouter<Self>,
}

impl MyServer {
    pub async fn new(pool: SqlitePool) -> Result<Self> {
        let mut router = ToolRouter::new();

        // Load tools from database
        let tools = sqlx::query!(
            "SELECT id, name, description, input_schema FROM tools"
        )
        .fetch_all(&pool)
        .await?;

        for tool in tools {
            let schema: serde_json::Value = serde_json::from_str(&tool.input_schema)?;

            let handler = DatabaseToolHandler {
                tool_id: tool.id,
                pool: pool.clone(),
            };

            router.register_dynamic_tool(
                tool.name,
                Some(tool.description),
                schema,
                Arc::new(handler),
            )?;
        }

        Ok(Self { pool, tool_router: router })
    }
}

// Handler that executes database-driven logic
struct DatabaseToolHandler {
    tool_id: i64,
    pool: SqlitePool,
}

impl DynamicToolHandler<MyServer> for DatabaseToolHandler {
    fn call(
        &self,
        service: &MyServer,
        params: Option<JsonObject>,
    ) -> BoxFuture<'static, Result<CallToolResult, rmcp::ErrorData>> {
        let tool_id = self.tool_id;
        let pool = self.pool.clone();

        Box::pin(async move {
            // Fetch tool configuration from database
            let config = sqlx::query!(
                "SELECT endpoint, method FROM tool_config WHERE tool_id = ?",
                tool_id
            )
            .fetch_one(&pool)
            .await
            .map_err(|e| rmcp::ErrorData::internal_error(
                format!("Database error: {}", e),
                None
            ))?;

            // Execute HTTP request or other logic
            let response = execute_request(&config, params).await?;

            Ok(CallToolResult::success(vec![Content::text(response)]))
        })
    }
}
```

### Example: Stateful Handler

```rust
use tokio::sync::RwLock;

struct CounterHandler {
    count: Arc<RwLock<i32>>,
}

impl<S> DynamicToolHandler<S> for CounterHandler {
    fn call(
        &self,
        _service: &S,
        _params: Option<JsonObject>,
    ) -> BoxFuture<'static, Result<CallToolResult, rmcp::ErrorData>> {
        let count = Arc::clone(&self.count);

        Box::pin(async move {
            let mut c = count.write().await;
            *c += 1;

            Ok(CallToolResult::success(vec![
                Content::text(format!("Count: {}", *c))
            ]))
        })
    }
}

// Usage
let counter = Arc::new(RwLock::new(0));
router.register_dynamic_tool(
    "counter".to_string(),
    Some("Increment counter".to_string()),
    json!({"type": "object", "properties": {}}),
    Arc::new(CounterHandler { count: counter }),
)?;
```

### Example: Runtime Management

```rust
use std::sync::{Arc, RwLock};

#[derive(Clone)]
pub struct DynamicServer {
    // Wrap router in Arc<RwLock<>> for runtime modifications
    router: Arc<RwLock<ToolRouter<Self>>>,
}

impl DynamicServer {
    pub fn new() -> Self {
        Self {
            router: Arc::new(RwLock::new(ToolRouter::new())),
        }
    }

    // Add tools at runtime
    pub fn add_tool(&self, name: String, handler: Arc<dyn DynamicToolHandler<Self>>)
        -> Result<(), ToolRegistrationError>
    {
        let schema = json!({"type": "object", "properties": {}});

        self.router.write().unwrap().register_dynamic_tool(
            name,
            None,
            schema,
            handler,
        )
    }

    // Remove tools at runtime
    pub fn remove_tool(&self, name: &str) -> Result<(), ToolNotFoundError> {
        self.router.write().unwrap().unregister_tool(name)
    }

    // List all tools
    pub fn list_tools(&self) -> Vec<String> {
        self.router.read().unwrap().tool_names()
    }
}

impl ServerHandler for DynamicServer {
    async fn call_tool(
        &self,
        request: CallToolRequestParam,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let tcc = ToolCallContext::new(self, request, context);
        self.router.read().unwrap().call(tcc).await
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParam>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let tools = self.router.read().unwrap().list_all();
        Ok(ListToolsResult::with_all_items(tools))
    }

    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            capabilities: ServerCapabilities::builder()
                .enable_tools()
                .build(),
            ..Default::default()
        }
    }
}
```

## Integration with Existing Code

### Backward Compatibility

✅ **100% backward compatible** - All existing macro-based tools continue to work unchanged.

```rust
// Existing macro-based tools still work
#[tool_router]
impl MyServer {
    #[tool(description = "Static tool")]
    async fn static_tool(&self, params: Parameters<MyParams>) -> String {
        "works!".to_string()
    }
}

// Mix with dynamic tools
let mut router = Self::tool_router(); // Get macro-generated router

router.register_dynamic_tool(
    "dynamic_tool".to_string(),
    Some("Dynamic tool".to_string()),
    schema,
    Arc::new(MyDynamicHandler),
)?;
```

### Combining Static and Dynamic Tools

```rust
#[derive(Clone)]
pub struct HybridServer {
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl HybridServer {
    // Static tool (compile-time)
    #[tool(description = "Get server status")]
    async fn status(&self) -> String {
        "Server is running".to_string()
    }

    // Static tool (compile-time)
    #[tool(description = "Ping the server")]
    async fn ping(&self) -> String {
        "pong".to_string()
    }
}

impl HybridServer {
    pub async fn new(database_url: &str) -> Result<Self> {
        // Start with static tools from macros
        let mut router = Self::tool_router();

        // Add dynamic tools from database
        let pool = SqlitePool::connect(database_url).await?;
        let dynamic_tools = load_tools_from_db(&pool).await?;

        for (name, handler) in dynamic_tools {
            router.register_dynamic_tool(
                name,
                None,
                json!({"type": "object", "properties": {}}),
                handler,
            )?;
        }

        Ok(Self { tool_router: router })
    }
}

impl ServerHandler for HybridServer {
    #[tool_handler] // Macro uses self.tool_router by default
    async fn call_tool(/* ... */) -> Result<CallToolResult, ErrorData>;

    #[tool_handler]
    async fn list_tools(/* ... */) -> Result<ListToolsResult, ErrorData>;

    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            capabilities: ServerCapabilities::builder()
                .enable_tools()
                .build(),
            ..Default::default()
        }
    }
}
```

## Error Handling

### Handling Registration Errors

```rust
use rmcp::model::ToolRegistrationError;

match router.register_dynamic_tool(name, desc, schema, handler) {
    Ok(()) => println!("Tool registered successfully"),
    Err(ToolRegistrationError::DuplicateTool(name)) => {
        eprintln!("Tool '{}' already exists", name);
    }
    Err(ToolRegistrationError::InvalidName(msg)) => {
        eprintln!("Invalid tool name: {}", msg);
    }
    Err(ToolRegistrationError::InvalidSchema(msg)) => {
        eprintln!("Invalid schema: {}", msg);
    }
}
```

### Handling Unregistration Errors

```rust
use rmcp::model::ToolNotFoundError;

match router.unregister_tool("my_tool") {
    Ok(()) => println!("Tool removed"),
    Err(ToolNotFoundError::NotFound(name)) => {
        eprintln!("Tool '{}' not found or is a static tool", name);
    }
}
```

### Handler Error Propagation

```rust
impl<S> DynamicToolHandler<S> for MyHandler {
    fn call(
        &self,
        _service: &S,
        params: Option<JsonObject>,
    ) -> BoxFuture<'static, Result<CallToolResult, rmcp::ErrorData>> {
        Box::pin(async move {
            // Validate parameters
            let params = params.ok_or_else(||
                rmcp::ErrorData::invalid_params("Missing parameters", None)
            )?;

            // Extract required field
            let value = params.get("required_field")
                .ok_or_else(|| rmcp::ErrorData::invalid_params(
                    "Missing required_field",
                    Some(serde_json::json!({"required": ["required_field"]}))
                ))?;

            // Process...
            let result = process(value).await
                .map_err(|e| rmcp::ErrorData::internal_error(
                    format!("Processing failed: {}", e),
                    None
                ))?;

            Ok(CallToolResult::success(vec![Content::text(result)]))
        })
    }
}
```

## JSON Schema Generation

For type-safe parameter handling, use `schemars`:

```rust
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, JsonSchema)]
struct MyToolParams {
    message: String,
    count: i32,
}

// Generate schema
let schema = schemars::schema_for!(MyToolParams);
let schema_json = serde_json::to_value(&schema)?;

router.register_dynamic_tool(
    "my_tool".to_string(),
    Some("My tool with typed params".to_string()),
    schema_json,
    Arc::new(handler),
)?;

// In handler, parse params
impl<S> DynamicToolHandler<S> for MyHandler {
    fn call(
        &self,
        _service: &S,
        params: Option<JsonObject>,
    ) -> BoxFuture<'static, Result<CallToolResult, rmcp::ErrorData>> {
        Box::pin(async move {
            let params_value = serde_json::to_value(params)
                .map_err(|e| rmcp::ErrorData::invalid_params(
                    format!("Invalid params: {}", e),
                    None
                ))?;

            let typed_params: MyToolParams = serde_json::from_value(params_value)
                .map_err(|e| rmcp::ErrorData::invalid_params(
                    format!("Failed to parse params: {}", e),
                    None
                ))?;

            // Use typed_params.message, typed_params.count...

            Ok(CallToolResult::success(vec![Content::text("Done")]))
        })
    }
}
```

## Testing

### Unit Testing Dynamic Tools

```rust
#[tokio::test]
async fn test_my_dynamic_tool() {
    let handler = MyDynamicHandler;
    let service = MyService::new();

    let params = serde_json::json!({
        "message": "test"
    }).as_object().unwrap().clone();

    let result = handler.call(&service, Some(params)).await;

    assert!(result.is_ok());
    let content = result.unwrap().content.first().unwrap();
    assert_eq!(content.as_text().unwrap().text, "test");
}
```

### Integration Testing

```rust
#[tokio::test]
async fn test_dynamic_registration_lifecycle() {
    let mut router = ToolRouter::new();

    // Register
    let result = router.register_dynamic_tool(
        "test".to_string(),
        None,
        json!({"type": "object", "properties": {}}),
        Arc::new(TestHandler),
    );
    assert!(result.is_ok());

    // Verify registered
    assert!(router.has_tool("test"));
    assert_eq!(router.dynamic_tool_count(), 1);

    // List tools
    let tools = router.list_all();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "test");

    // Unregister
    let result = router.unregister_tool("test");
    assert!(result.is_ok());
    assert!(!router.has_tool("test"));
}
```

## API Reference

### DynamicToolHandler Trait

```rust
pub trait DynamicToolHandler<S>: Send + Sync + 'static {
    fn call(
        &self,
        service: &S,
        params: Option<JsonObject>,
    ) -> BoxFuture<'static, Result<CallToolResult, ErrorData>>;
}
```

**Type Parameters:**
- `S` - Service type (your server struct)

**Requirements:**
- Must be `Send + Sync + 'static` for thread safety
- Must return `BoxFuture<'static, ...>` to avoid lifetime issues

### ToolRouter Methods

#### `register_dynamic_tool`

```rust
pub fn register_dynamic_tool(
    &mut self,
    name: String,
    description: Option<String>,
    input_schema: serde_json::Value,
    handler: Arc<dyn DynamicToolHandler<S>>,
) -> Result<(), ToolRegistrationError>
```

**Parameters:**
- `name` - Unique tool identifier (must not be empty)
- `description` - Optional human-readable description
- `input_schema` - JSON Schema for parameters (must be object type)
- `handler` - Handler implementation wrapped in `Arc`

**Returns:**
- `Ok(())` - Tool registered successfully
- `Err(ToolRegistrationError)` - Registration failed

**Errors:**
- `DuplicateTool` - Tool with this name already exists
- `InvalidName` - Empty or invalid tool name
- `InvalidSchema` - Schema is not a valid JSON object

#### `unregister_tool`

```rust
pub fn unregister_tool(&mut self, name: &str) -> Result<(), ToolNotFoundError>
```

**Parameters:**
- `name` - Tool name to remove

**Returns:**
- `Ok(())` - Tool removed successfully
- `Err(ToolNotFoundError)` - Tool not found or is static

**Safety:** Only dynamic tools can be unregistered. Static tools (from macros) cannot be removed.

#### `has_tool`

```rust
pub fn has_tool(&self, name: &str) -> bool
```

Check if a tool exists (static or dynamic).

#### `tool_names`

```rust
pub fn tool_names(&self) -> Vec<String>
```

Get all tool names (static + dynamic).

#### `dynamic_tool_count`

```rust
pub fn dynamic_tool_count(&self) -> usize
```

Count of dynamically registered tools.

#### `static_tool_count`

```rust
pub fn static_tool_count(&self) -> usize
```

Count of statically registered tools (from macros).

## Performance Considerations

### Memory

- **Arc overhead:** Minimal - shared pointers, not data duplication
- **HashMap lookup:** O(1) average case for tool dispatch
- **Clone cost:** Cheap - just reference count increment

### Concurrency

For thread-safe runtime modifications, wrap the router in `Arc<RwLock<>>`:

```rust
use std::sync::{Arc, RwLock};

pub struct MyServer {
    router: Arc<RwLock<ToolRouter<Self>>>,
}
```

**Pattern:**
- **RwLock:** Allows concurrent reads (`list_tools`, `call_tool`)
- **Write locks:** Only during registration/unregistration
- **Read-heavy workload:** Optimal for typical MCP usage

## Migration Guide

### From Static-Only to Dynamic

**Before (static only):**

```rust
#[tool_router]
impl MyServer {
    #[tool]
    async fn my_tool(&self, params: Parameters<MyParams>) -> String {
        "result".to_string()
    }
}
```

**After (with dynamic tools):**

```rust
impl MyServer {
    pub async fn new() -> Self {
        let mut router = Self::tool_router(); // Keep static tools

        // Add dynamic tools
        router.register_dynamic_tool(
            "dynamic_tool".to_string(),
            Some("A dynamic tool".to_string()),
            schema,
            Arc::new(MyDynamicHandler),
        ).unwrap();

        Self { tool_router: router }
    }
}
```

### From Database Configuration

If you're migrating from a system where tools were hard-coded but should come from a database:

```rust
// Old: Hard-coded tools
#[tool_router]
impl MyServer {
    #[tool]
    async fn api_call_1(&self) -> String { /* ... */ }

    #[tool]
    async fn api_call_2(&self) -> String { /* ... */ }
}

// New: Database-driven tools
impl MyServer {
    pub async fn new(db: Database) -> Self {
        let mut router = ToolRouter::new();

        let apis = db.query("SELECT * FROM api_configs").await?;

        for api in apis {
            router.register_dynamic_tool(
                api.name,
                Some(api.description),
                api.schema,
                Arc::new(GenericApiHandler::new(api)),
            )?;
        }

        Self { tool_router: router }
    }
}
```

## Best Practices

### 1. Use Type-Safe Schemas

Generate JSON schemas from Rust types using `schemars`:

```rust
use schemars::JsonSchema;

#[derive(JsonSchema)]
struct MyParams {
    field: String,
}

let schema = schemars::schema_for!(MyParams);
```

### 2. Handle Errors Properly

Never use `unwrap()` or `expect()` in handler implementations:

```rust
// ❌ Bad
let value = params.get("key").unwrap();

// ✅ Good
let value = params.get("key")
    .ok_or_else(|| ErrorData::invalid_params("Missing key", None))?;
```

### 3. Keep Handlers Focused

Each handler should do one thing well:

```rust
// ✅ Good - Single responsibility
struct GetUserHandler { db: Pool }
struct CreateUserHandler { db: Pool }
struct DeleteUserHandler { db: Pool }

// ❌ Bad - Too generic
struct UniversalUserHandler { db: Pool } // Handles all user operations
```

### 4. Use Arc for Shared State

When handlers need shared state, use `Arc`:

```rust
struct MyHandler {
    config: Arc<Config>,
    db: Arc<Pool>,
}

impl Clone for MyHandler {
    fn clone(&self) -> Self {
        Self {
            config: Arc::clone(&self.config),
            db: Arc::clone(&self.db),
        }
    }
}
```

### 5. Validate Early

Validate inputs at the start of your handler:

```rust
Box::pin(async move {
    // Validate first
    let params = params.ok_or_else(||
        ErrorData::invalid_params("Params required", None)
    )?;

    // Then process
    let result = process(params).await?;

    Ok(CallToolResult::success(vec![Content::text(result)]))
})
```

## Limitations and Known Issues

### 1. Tool Name Conflicts

Once a tool is registered (static or dynamic), the name is reserved. You must unregister before re-registering with the same name.

### 2. Static Tools Cannot Be Unregistered

Tools registered via macros (`#[tool]`) cannot be removed at runtime. This is by design for safety.

### 3. No Automatic Schema Validation

The library doesn't validate that handler parameters match the input schema at runtime. This is the developer's responsibility.

### 4. Thread Safety Requires Arc<RwLock<>>

For runtime registration/unregistration in a multi-threaded server, you must wrap the router:

```rust
router: Arc<RwLock<ToolRouter<Self>>>
```

## Changelog

### Version 0.8.0-dynamic.1 (Current)

**Added:**
- `DynamicToolHandler` trait for runtime tool implementations
- `register_dynamic_tool()` method on `ToolRouter`
- `unregister_tool()` method on `ToolRouter`
- `has_tool()`, `tool_names()`, `dynamic_tool_count()`, `static_tool_count()` helper methods
- `ToolRegistrationError` and `ToolNotFoundError` error types
- Comprehensive test suite (15 tests)

**Changed:**
- `ToolRouter` now tracks dynamic vs static tools separately
- Extended `Clone` implementation to include dynamic tools

**Backward Compatibility:**
- ✅ 100% compatible with existing macro-based tools
- ✅ All existing tests pass
- ✅ No breaking changes to public API

## Support and Contributing

### Questions?

- Check the [examples](tests/test_dynamic_tools.rs) in the test suite
- Review the [main documentation](README.md)
- Open an issue on GitHub

### Contributing

Contributions are welcome! Please ensure:
- All tests pass (`cargo test --all-features`)
- No clippy warnings (`cargo clippy --all-features -- -D warnings`)
- Code is formatted (`cargo fmt`)
- New features include tests and documentation

## License

This fork maintains the same license as the original rmcp project.

---

**Last Updated:** 2025-10-05
**Implemented By:** rmcp-ultra team
