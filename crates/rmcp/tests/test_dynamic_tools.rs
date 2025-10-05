// Test file for dynamic tool registration functionality
use futures::future::BoxFuture;
use rmcp::handler::server::router::tool::{DynamicToolHandler, ToolRouter};
use rmcp::model::{CallToolResult, Content, JsonObject, ToolNotFoundError, ToolRegistrationError};
use serde_json::json;
use std::sync::Arc;

// Test service for dynamic tools
#[derive(Clone)]
struct TestService {
    name: String,
}

// Simple echo handler that returns the input message
struct EchoHandler;

impl DynamicToolHandler<TestService> for EchoHandler {
    fn call(
        &self,
        _service: &TestService,
        params: Option<JsonObject>,
    ) -> BoxFuture<'static, Result<CallToolResult, rmcp::ErrorData>> {
        Box::pin(async move {
            let message = params
                .as_ref()
                .and_then(|p| p.get("message"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| "default".to_string());

            Ok(CallToolResult::success(vec![Content::text(message)]))
        })
    }
}

// Handler that uses service state
struct ServiceStateHandler;

impl DynamicToolHandler<TestService> for ServiceStateHandler {
    fn call(
        &self,
        service: &TestService,
        _params: Option<JsonObject>,
    ) -> BoxFuture<'static, Result<CallToolResult, rmcp::ErrorData>> {
        let service_name = service.name.clone();
        Box::pin(async move {
            Ok(CallToolResult::success(vec![Content::text(format!(
                "Service: {}",
                service_name
            ))]))
        })
    }
}

// Handler that returns errors
struct ErrorHandler;

impl DynamicToolHandler<TestService> for ErrorHandler {
    fn call(
        &self,
        _service: &TestService,
        _params: Option<JsonObject>,
    ) -> BoxFuture<'static, Result<CallToolResult, rmcp::ErrorData>> {
        Box::pin(async move { Err(rmcp::ErrorData::invalid_params("Intentional error", None)) })
    }
}

#[test]
fn test_register_dynamic_tool_success() {
    let mut router: ToolRouter<TestService> = ToolRouter::new();

    let schema = json!({
        "type": "object",
        "properties": {
            "message": {"type": "string"}
        }
    });

    let result = router.register_dynamic_tool(
        "echo".to_string(),
        Some("Echo a message".to_string()),
        schema,
        Arc::new(EchoHandler),
    );

    assert!(result.is_ok());
    assert!(router.has_tool("echo"));
    assert_eq!(router.dynamic_tool_count(), 1);
    assert_eq!(router.static_tool_count(), 0);
}

#[test]
fn test_register_duplicate_tool_fails() {
    let mut router: ToolRouter<TestService> = ToolRouter::new();

    let schema = json!({
        "type": "object",
        "properties": {}
    });

    // First registration should succeed
    let result1 = router.register_dynamic_tool(
        "echo".to_string(),
        Some("Echo tool".to_string()),
        schema.clone(),
        Arc::new(EchoHandler),
    );
    assert!(result1.is_ok());

    // Second registration with same name should fail
    let result2 = router.register_dynamic_tool(
        "echo".to_string(),
        Some("Another echo".to_string()),
        schema,
        Arc::new(EchoHandler),
    );

    assert!(result2.is_err());
    match result2.unwrap_err() {
        ToolRegistrationError::DuplicateTool(name) => {
            assert_eq!(name, "echo");
        }
        _ => panic!("Expected DuplicateTool error"),
    }
}

#[test]
fn test_register_empty_name_fails() {
    let mut router: ToolRouter<TestService> = ToolRouter::new();

    let schema = json!({"type": "object", "properties": {}});

    let result = router.register_dynamic_tool(
        "".to_string(),
        Some("Tool".to_string()),
        schema,
        Arc::new(EchoHandler),
    );

    assert!(result.is_err());
    match result.unwrap_err() {
        ToolRegistrationError::InvalidName(_) => {}
        _ => panic!("Expected InvalidName error"),
    }
}

#[test]
fn test_register_invalid_schema_fails() {
    let mut router: ToolRouter<TestService> = ToolRouter::new();

    // Schema must be an object, not a string
    let schema = json!("not an object");

    let result = router.register_dynamic_tool(
        "test".to_string(),
        Some("Test".to_string()),
        schema,
        Arc::new(EchoHandler),
    );

    assert!(result.is_err());
    match result.unwrap_err() {
        ToolRegistrationError::InvalidSchema(_) => {}
        _ => panic!("Expected InvalidSchema error"),
    }
}

#[test]
fn test_unregister_dynamic_tool_success() {
    let mut router: ToolRouter<TestService> = ToolRouter::new();

    let schema = json!({"type": "object", "properties": {}});

    router
        .register_dynamic_tool(
            "echo".to_string(),
            Some("Echo".to_string()),
            schema,
            Arc::new(EchoHandler),
        )
        .unwrap();

    assert!(router.has_tool("echo"));

    let result = router.unregister_tool("echo");
    assert!(result.is_ok());
    assert!(!router.has_tool("echo"));
    assert_eq!(router.dynamic_tool_count(), 0);
}

#[test]
fn test_unregister_nonexistent_tool_fails() {
    let mut router: ToolRouter<TestService> = ToolRouter::new();

    let result = router.unregister_tool("nonexistent");

    assert!(result.is_err());
    match result.unwrap_err() {
        ToolNotFoundError::NotFound(name) => {
            assert_eq!(name, "nonexistent");
        }
    }
}

#[test]
fn test_tool_names() {
    let mut router: ToolRouter<TestService> = ToolRouter::new();

    let schema = json!({"type": "object", "properties": {}});

    router
        .register_dynamic_tool(
            "tool1".to_string(),
            None,
            schema.clone(),
            Arc::new(EchoHandler),
        )
        .unwrap();

    router
        .register_dynamic_tool(
            "tool2".to_string(),
            None,
            schema.clone(),
            Arc::new(EchoHandler),
        )
        .unwrap();

    let names = router.tool_names();
    assert_eq!(names.len(), 2);
    assert!(names.contains(&"tool1".to_string()));
    assert!(names.contains(&"tool2".to_string()));
}

#[test]
fn test_has_tool() {
    let mut router: ToolRouter<TestService> = ToolRouter::new();

    let schema = json!({"type": "object", "properties": {}});

    assert!(!router.has_tool("echo"));

    router
        .register_dynamic_tool("echo".to_string(), None, schema, Arc::new(EchoHandler))
        .unwrap();

    assert!(router.has_tool("echo"));
}

#[test]
fn test_dynamic_and_static_counts() {
    let mut router: ToolRouter<TestService> = ToolRouter::new();

    assert_eq!(router.dynamic_tool_count(), 0);
    assert_eq!(router.static_tool_count(), 0);

    let schema = json!({"type": "object", "properties": {}});

    router
        .register_dynamic_tool(
            "dynamic1".to_string(),
            None,
            schema.clone(),
            Arc::new(EchoHandler),
        )
        .unwrap();

    router
        .register_dynamic_tool("dynamic2".to_string(), None, schema, Arc::new(EchoHandler))
        .unwrap();

    assert_eq!(router.dynamic_tool_count(), 2);
    assert_eq!(router.static_tool_count(), 0);
}

#[test]
fn test_full_dynamic_lifecycle() {
    let mut router: ToolRouter<TestService> = ToolRouter::new();

    // 1. Initially no tools
    assert_eq!(router.tool_names().len(), 0);

    // 2. Register a dynamic tool
    let schema = json!({
        "type": "object",
        "properties": {
            "message": {"type": "string"}
        }
    });

    router
        .register_dynamic_tool(
            "echo".to_string(),
            Some("Echo a message".to_string()),
            schema,
            Arc::new(EchoHandler),
        )
        .unwrap();

    // 3. Verify tool is registered
    assert!(router.has_tool("echo"));
    let tools = router.list_all();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "echo");
    assert_eq!(
        tools[0].description.as_ref().map(|s| s.as_ref()),
        Some("Echo a message")
    );

    // 4. Unregister the tool
    router.unregister_tool("echo").unwrap();
    assert!(!router.has_tool("echo"));
    assert_eq!(router.tool_names().len(), 0);
}

#[tokio::test]
async fn test_dynamic_handler_execution() {
    // Test that the handler can be called directly
    let handler = EchoHandler;
    let service = TestService {
        name: "test".to_string(),
    };

    let mut params = JsonObject::new();
    params.insert("message".to_string(), json!("test message"));

    let result = handler.call(&service, Some(params)).await;
    assert!(result.is_ok());

    let call_result = result.unwrap();
    let content = call_result.content.first().expect("Expected content");
    let text_content = content.as_text().expect("Expected text content");
    assert_eq!(text_content.text.as_str(), "test message");
}

#[tokio::test]
async fn test_service_state_handler() {
    let handler = ServiceStateHandler;
    let service = TestService {
        name: "MyService".to_string(),
    };

    let result = handler.call(&service, None).await;
    assert!(result.is_ok());

    let call_result = result.unwrap();
    let content = call_result.content.first().expect("Expected content");
    let text_content = content.as_text().expect("Expected text content");
    assert_eq!(text_content.text.as_str(), "Service: MyService");
}

#[tokio::test]
async fn test_error_handler() {
    let handler = ErrorHandler;
    let service = TestService {
        name: "test".to_string(),
    };

    let result = handler.call(&service, None).await;
    assert!(result.is_err());

    if let Err(err) = result {
        assert!(err.message.contains("Intentional error"));
    }
}

#[test]
fn test_register_multiple_tools() {
    let mut router: ToolRouter<TestService> = ToolRouter::new();

    let schema = json!({"type": "object", "properties": {}});

    // Register multiple tools
    for i in 0..5 {
        router
            .register_dynamic_tool(
                format!("tool_{}", i),
                Some(format!("Tool number {}", i)),
                schema.clone(),
                Arc::new(EchoHandler),
            )
            .unwrap();
    }

    assert_eq!(router.dynamic_tool_count(), 5);
    assert_eq!(router.tool_names().len(), 5);

    // Unregister some tools
    router.unregister_tool("tool_1").unwrap();
    router.unregister_tool("tool_3").unwrap();

    assert_eq!(router.dynamic_tool_count(), 3);
    assert!(router.has_tool("tool_0"));
    assert!(!router.has_tool("tool_1"));
    assert!(router.has_tool("tool_2"));
    assert!(!router.has_tool("tool_3"));
    assert!(router.has_tool("tool_4"));
}

#[test]
fn test_clone_router_with_dynamic_tools() {
    let mut router: ToolRouter<TestService> = ToolRouter::new();

    let schema = json!({"type": "object", "properties": {}});

    router
        .register_dynamic_tool("echo".to_string(), None, schema, Arc::new(EchoHandler))
        .unwrap();

    let cloned = router.clone();

    assert!(cloned.has_tool("echo"));
    assert_eq!(cloned.dynamic_tool_count(), 1);
    assert_eq!(router.dynamic_tool_count(), 1);
}
