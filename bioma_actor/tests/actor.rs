use bioma_actor::prelude::*;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use surrealdb::sql;
use test_log::test;
use tokio::time::{sleep, Duration};
use tracing::{error, info};

// Custom error type for test actors
#[derive(Debug, thiserror::Error)]
enum TestError {
    #[error("System error: {0}")]
    System(#[from] SystemActorError),
    #[error("Fake error")]
    FakeError,
}

impl ActorError for TestError {}

// Test message types
#[derive(Clone, Debug, Serialize, Deserialize)]
struct TestMessage {
    content: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct TestResponse {
    content: String,
    count: usize,
}

// Test actor for basic message handling
#[derive(Debug, Serialize, Deserialize)]
struct TestActor {
    count: usize,
}

impl Message<TestMessage> for TestActor {
    type Response = TestResponse;

    async fn handle(&mut self, ctx: &mut ActorContext<Self>, msg: &TestMessage) -> Result<(), TestError> {
        self.count += 1;
        let response = TestResponse { content: format!("Received: {}", msg.content), count: self.count };
        ctx.reply(response).await?;
        Ok(())
    }
}

impl Actor for TestActor {
    type Error = TestError;

    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), TestError> {
        let mut stream = ctx.recv().await?;
        while let Some(Ok(frame)) = stream.next().await {
            if let Some(msg) = frame.is::<TestMessage>() {
                self.reply(ctx, &msg, &frame).await?;
            }
        }
        Ok(())
    }
}

// Additional actor and message types for error handling test
#[derive(Debug, Serialize, Deserialize)]
struct ErrorActor;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct TriggerError;

impl Message<TriggerError> for ErrorActor {
    type Response = ();

    async fn handle(&mut self, _ctx: &mut ActorContext<Self>, _: &TriggerError) -> Result<(), TestError> {
        Err(TestError::FakeError)
    }
}

impl Actor for ErrorActor {
    type Error = TestError;

    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), TestError> {
        let mut stream = ctx.recv().await?;
        while let Some(Ok(frame)) = stream.next().await {
            if let Some(trigger) = frame.is::<TriggerError>() {
                self.reply(ctx, &trigger, &frame).await?;
            }
        }
        Ok(())
    }
}

#[test(tokio::test)]
async fn test_actor_health() -> Result<(), TestError> {
    let engine = Engine::test().await?;

    let test_actor_id = ActorId::of::<TestActor>("/test");
    let (test_actor_ctx, _test_actor) =
        Actor::spawn(engine.clone(), test_actor_id.clone(), TestActor { count: 0 }, SpawnOptions::default()).await?;

    assert!(test_actor_ctx.health().await);

    // Terminate the actor
    test_actor_ctx.kill().await?;

    assert!(!test_actor_ctx.health().await);

    Ok(())
}

#[test(tokio::test)]
async fn test_actor_message_handling() -> Result<(), TestError> {
    let engine = Engine::test().await?;

    let test_actor_id = ActorId::of::<TestActor>("/test");
    let (mut test_actor_ctx, mut test_actor) =
        Actor::spawn(engine.clone(), test_actor_id.clone(), TestActor { count: 0 }, SpawnOptions::default()).await?;

    let test_handle = tokio::spawn(async move {
        if let Err(e) = test_actor.start(&mut test_actor_ctx).await {
            eprintln!("TestActor error: {}", e);
        }
    });

    let relay_actor_id = ActorId::of::<Relay>("/relay");
    let (relay_actor_ctx, _relay_actor) =
        Actor::spawn(engine.clone(), relay_actor_id.clone(), Relay, SpawnOptions::default()).await?;

    // Send a message and collect response from stream
    let message = TestMessage { content: "Hello, Actor!".to_string() };
    let mut response_stream =
        relay_actor_ctx.send::<TestActor, TestMessage>(message, &test_actor_id, SendOptions::default()).await?;

    // Get first (and only) response from stream
    if let Some(Ok(response)) = response_stream.next().await {
        info!("Received response: {:?}", response);
        assert_eq!(response.content, "Received: Hello, Actor!");
        assert_eq!(response.count, 1);
    } else {
        panic!("No response received");
    }

    // Terminate the actor
    test_handle.abort();

    dbg_export_db!(engine);

    Ok(())
}

#[test(tokio::test)]
async fn test_actor_multiple_messages() -> Result<(), TestError> {
    let engine = Engine::test().await?;

    let test_actor_id = ActorId::of::<TestActor>("/test");
    let (mut test_actor_ctx, mut test_actor) =
        Actor::spawn(engine.clone(), test_actor_id.clone(), TestActor { count: 0 }, SpawnOptions::default()).await?;

    let test_handle = tokio::spawn(async move {
        if let Err(e) = test_actor.start(&mut test_actor_ctx).await {
            eprintln!("TestActor error: {}", e);
        }
    });

    let relay_actor_id = ActorId::of::<Relay>("/relay");
    let (relay_actor_ctx, _relay_actor) =
        Actor::spawn(engine.clone(), relay_actor_id.clone(), Relay, SpawnOptions::default()).await?;

    // Send multiple messages
    for i in 1..=5 {
        let message = TestMessage { content: format!("Message {}", i) };
        let mut response_stream =
            relay_actor_ctx.send::<TestActor, TestMessage>(message, &test_actor_id, SendOptions::default()).await?;

        if let Some(Ok(response)) = response_stream.next().await {
            info!("Received response: {:?}", response);
            assert_eq!(response.content, format!("Received: Message {}", i));
            assert_eq!(response.count, i);
        } else {
            panic!("No response received for message {}", i);
        }
    }

    // Terminate the actor
    test_handle.abort();

    dbg_export_db!(engine);
    Ok(())
}

#[test(tokio::test)]
async fn test_actor_lifecycle() -> Result<(), TestError> {
    let engine = Engine::test().await?;

    let test_actor_id = ActorId::of::<TestActor>("/test");
    let (mut test_actor_ctx, mut test_actor) =
        Actor::spawn(engine.clone(), test_actor_id.clone(), TestActor { count: 0 }, SpawnOptions::default()).await?;

    let test_handle = tokio::spawn(async move {
        if let Err(e) = test_actor.start(&mut test_actor_ctx).await {
            eprintln!("TestActor error: {}", e);
        }
    });

    let relay_actor_id = ActorId::of::<Relay>("/relay");
    let (relay_actor_ctx, _relay_actor) =
        Actor::spawn(engine.clone(), relay_actor_id.clone(), Relay, SpawnOptions::default()).await?;

    // Send a message to ensure the actor is working
    let message = TestMessage { content: "Lifecycle test".to_string() };
    let mut response_stream =
        relay_actor_ctx.send::<TestActor, TestMessage>(message, &test_actor_id, SendOptions::default()).await?;

    if let Some(Ok(response)) = response_stream.next().await {
        info!("Received response: {:?}", response);
        assert_eq!(response.content, "Received: Lifecycle test");
        assert_eq!(response.count, 1);
    } else {
        panic!("No response received");
    }

    // Terminate the actor
    test_handle.abort();
    sleep(Duration::from_millis(100)).await;

    // Try to send a message to the terminated actor
    let message = TestMessage { content: "After termination".to_string() };
    let options = SendOptions::builder().timeout(Duration::from_secs(1)).build();
    let result = relay_actor_ctx.send_and_wait_reply::<TestActor, TestMessage>(message, &test_actor_id, options).await;

    assert!(result.is_err());

    dbg_export_db!(engine);
    Ok(())
}

#[test(tokio::test)]
async fn test_actor_error_handling() -> Result<(), TestError> {
    let engine = Engine::test().await?;

    let error_actor_id = ActorId::of::<ErrorActor>("/error_actor");
    let (mut error_actor_ctx, mut error_actor) =
        Actor::spawn(engine.clone(), error_actor_id.clone(), ErrorActor, SpawnOptions::default()).await?;

    let error_handle = tokio::spawn(async move {
        if let Err(e) = error_actor.start(&mut error_actor_ctx).await {
            assert!(e.to_string().contains("Fake error"));
        }
    });

    let relay_actor_id = ActorId::of::<Relay>("/relay");
    let (relay_actor_ctx, _relay_actor) =
        Actor::spawn(engine.clone(), relay_actor_id.clone(), Relay, SpawnOptions::default()).await?;

    // Trigger the error
    let mut response_stream =
        relay_actor_ctx.send::<ErrorActor, TriggerError>(TriggerError, &error_actor_id, SendOptions::default()).await?;

    // Should receive error or no response
    let result = response_stream.next().await;
    assert!(result.is_none() || result.unwrap().is_err());

    // Wait for actor to finish
    let _ = error_handle.await;

    dbg_export_db!(engine);
    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
struct StatefulActor {
    count: u32,
}

impl Actor for StatefulActor {
    type Error = SystemActorError;

    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), Self::Error> {
        info!("{} Started with count: {}", ctx.id(), self.count);
        let mut stream = ctx.recv().await?;
        while let Some(Ok(frame)) = stream.next().await {
            if let Some(msg) = frame.is::<IncrementCount>() {
                self.reply(ctx, &msg, &frame).await?;
            } else if let Some(msg) = frame.is::<LargeMessage>() {
                self.reply(ctx, &msg, &frame).await?;
            }
            self.save(ctx).await?;
        }
        Ok(())
    }
}

impl Message<IncrementCount> for StatefulActor {
    type Response = u32;

    async fn handle(&mut self, ctx: &mut ActorContext<Self>, _msg: &IncrementCount) -> Result<(), Self::Error> {
        self.count += 1;
        ctx.reply(self.count).await?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct IncrementCount;

#[test(tokio::test)]
async fn test_actor_state_persistence() -> Result<(), TestError> {
    let engine = Engine::test().await?;

    let actor_id = ActorId::of::<StatefulActor>("/stateful_actor");

    // Spawn the actor
    let (mut actor_ctx, mut actor) =
        Actor::spawn(engine.clone(), actor_id.clone(), StatefulActor { count: 0 }, SpawnOptions::default()).await?;

    // Start the actor
    let actor_handle = tokio::spawn(async move {
        if let Err(e) = actor.start(&mut actor_ctx).await {
            error!("StatefulActor error: {}", e);
        }
    });

    // Create a relay actor to send messages
    let relay_id = ActorId::of::<Relay>("/relay");
    let (relay_actor_ctx, _relay_actor) =
        Actor::spawn(engine.clone(), relay_id.clone(), Relay, SpawnOptions::default()).await?;

    // Increment the count and get response
    let mut response_stream = relay_actor_ctx
        .send::<StatefulActor, IncrementCount>(IncrementCount, &actor_id, SendOptions::default())
        .await?;

    if let Some(Ok(count)) = response_stream.next().await {
        assert_eq!(count, 1);
    } else {
        panic!("No response received");
    }

    // Terminate the actor
    actor_handle.abort();
    sleep(Duration::from_millis(100)).await;

    // Respawn the actor with restore option
    let (mut restored_actor_ctx, mut restored_actor) = Actor::spawn(
        engine.clone(),
        actor_id.clone(),
        StatefulActor { count: 0 }, // This initial state should be overwritten
        SpawnOptions::builder().exists(SpawnExistsOptions::Restore).build(),
    )
    .await?;

    // Start the restored actor
    let restored_handle = tokio::spawn(async move {
        if let Err(e) = restored_actor.start(&mut restored_actor_ctx).await {
            error!("Restored StatefulActor error: {}", e);
        }
    });

    // Increment the count again
    let mut response_stream = relay_actor_ctx
        .send::<StatefulActor, IncrementCount>(IncrementCount, &actor_id, SendOptions::default())
        .await?;

    let response = if let Some(Ok(count)) = response_stream.next().await {
        count
    } else {
        panic!("No response received");
    };
    assert_eq!(response, 2);

    // Terminate the restored actor
    restored_handle.abort();

    dbg_export_db!(engine);

    Ok(())
}

use rand::Rng;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LargeMessage {
    data: Vec<u8>,
}

impl Message<LargeMessage> for StatefulActor {
    type Response = usize;

    async fn handle(&mut self, ctx: &mut ActorContext<Self>, msg: &LargeMessage) -> Result<(), Self::Error> {
        ctx.reply(msg.data.len()).await?;
        Ok(())
    }
}

#[test(tokio::test)]
#[ignore = "This test uses large messages and should only be run explicitly"]
async fn test_actor_large_message_mem_db() -> Result<(), TestError> {
    let engine = Engine::test().await?;

    // Create a large message (5MB of random data)
    let mut rng = rand::thread_rng();
    let large_data: Vec<u8> = (0..5_000_000).map(|_| rng.gen()).collect();
    let large_message = LargeMessage { data: large_data.clone() };

    // Spawn the stateful actor
    let stateful_actor_id = ActorId::of::<StatefulActor>("/large_message_actor");
    let (mut stateful_actor_ctx, mut stateful_actor) =
        Actor::spawn(engine.clone(), stateful_actor_id.clone(), StatefulActor { count: 0 }, SpawnOptions::default())
            .await?;

    // Start the stateful actor
    let stateful_actor_handle = tokio::spawn(async move {
        if let Err(e) = stateful_actor.start(&mut stateful_actor_ctx).await {
            error!("StatefulActor error: {}", e);
        }
    });

    // Create a relay actor to send messages
    let relay_id = ActorId::of::<Relay>("/relay");
    let (relay_actor_ctx, _relay_actor) =
        Actor::spawn(engine.clone(), relay_id.clone(), Relay, SpawnOptions::default()).await?;

    // Send the large message
    let mut response_stream = relay_actor_ctx
        .send::<StatefulActor, LargeMessage>(large_message, &stateful_actor_id, SendOptions::default())
        .await?;

    // Verify the response
    if let Some(Ok(response)) = response_stream.next().await {
        assert_eq!(response, 5_000_000);
    } else {
        panic!("No response received");
    }

    // Terminate the stateful actor
    stateful_actor_handle.abort();

    dbg_export_db!(engine);

    Ok(())
}

#[test(tokio::test)]
#[ignore = "This test uses large messages and should only be run explicitly"]
async fn test_actor_large_message_db() -> Result<(), TestError> {
    let engine_options = EngineOptions::builder().endpoint("ws://localhost:9123".into()).build();
    let engine = Engine::connect(engine_options).await?;

    let msg_size = 200_000;

    // Create a large message of random data
    let mut rng = rand::thread_rng();
    let large_data: Vec<u8> = (0..msg_size).map(|_| rng.gen()).collect();
    let large_message = LargeMessage { data: large_data.clone() };

    // Spawn the stateful actor
    let stateful_actor_id = ActorId::of::<StatefulActor>("/large_message_actor");
    let (mut stateful_actor_ctx, mut stateful_actor) =
        Actor::spawn(engine.clone(), stateful_actor_id.clone(), StatefulActor { count: 0 }, SpawnOptions::default())
            .await?;

    // Start the stateful actor
    let stateful_actor_handle = tokio::spawn(async move {
        if let Err(e) = stateful_actor.start(&mut stateful_actor_ctx).await {
            error!("StatefulActor error: {}", e);
        }
    });

    // Create a relay actor to send messages
    let relay_id = ActorId::of::<Relay>("/relay");
    let (relay_actor_ctx, _relay_actor) = Actor::spawn(
        engine.clone(),
        relay_id.clone(),
        Relay,
        SpawnOptions::builder().exists(SpawnExistsOptions::Reset).build(),
    )
    .await?;

    // Send the large message
    let mut response_stream = relay_actor_ctx
        .send::<StatefulActor, LargeMessage>(large_message, &stateful_actor_id, SendOptions::default())
        .await?;

    let response = if let Some(Ok(size)) = response_stream.next().await {
        size
    } else {
        panic!("No response received");
    };
    assert_eq!(response, msg_size);

    // Terminate the stateful actor
    stateful_actor_handle.abort();

    Ok(())
}

#[test(tokio::test)]
async fn test_actor_streaming_messages() -> Result<(), Box<dyn std::error::Error>> {
    let engine = Engine::test().await?;

    // Simple streaming actor that generates numbered messages
    #[derive(Debug, Serialize, Deserialize)]
    struct StreamingActor {
        message_count: usize,
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    struct StreamRequest {
        count: usize,
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    struct StreamResponse {
        part: usize,
        content: String,
    }

    impl Actor for StreamingActor {
        type Error = TestError;

        async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), Self::Error> {
            let mut stream = ctx.recv().await?;
            while let Some(Ok(frame)) = stream.next().await {
                if let Some(msg) = frame.is::<StreamRequest>() {
                    self.reply(ctx, &msg, &frame).await?;
                }
            }
            Ok(())
        }
    }

    impl Message<StreamRequest> for StreamingActor {
        type Response = StreamResponse;

        async fn handle(&mut self, ctx: &mut ActorContext<Self>, msg: &StreamRequest) -> Result<(), TestError> {
            for i in 0..msg.count {
                ctx.reply(StreamResponse { part: i + 1, content: format!("Message part {}", i + 1) }).await?;
                sleep(Duration::from_millis(50)).await;
            }
            Ok(())
        }
    }

    // Spawn the streaming actor
    let actor_id = ActorId::of::<StreamingActor>("/test/streamer");
    let (mut actor_ctx, mut actor) =
        Actor::spawn(engine.clone(), actor_id.clone(), StreamingActor { message_count: 0 }, SpawnOptions::default())
            .await?;

    // Start actor in background
    let actor_handle = tokio::spawn(async move {
        if let Err(e) = actor.start(&mut actor_ctx).await {
            error!("Streaming actor error: {}", e);
        }
    });

    // Create a new context to send messages
    let (sender_ctx, _) = Actor::spawn(
        engine.clone(),
        ActorId::of::<StreamingActor>("/test/sender"),
        StreamingActor { message_count: 0 },
        SpawnOptions::default(),
    )
    .await?;

    // Send request and collect responses
    let mut responses = Vec::new();
    let mut stream = sender_ctx
        .send::<StreamingActor, StreamRequest>(StreamRequest { count: 3 }, &actor_id, SendOptions::default())
        .await?;

    while let Some(response) = stream.next().await {
        responses.push(response?);
    }

    // Verify responses
    assert_eq!(responses.len(), 3);
    for (i, response) in responses.iter().enumerate() {
        assert_eq!(response.part, i + 1);
        assert_eq!(response.content, format!("Message part {}", i + 1));
    }

    actor_handle.abort();
    Ok(())
}

#[test(tokio::test)]
async fn test_health_monitoring_enabled() -> Result<(), TestError> {
    let engine = Engine::test().await?;

    // Create health config with short update interval
    let health_config = HealthConfig::builder().update_interval(sql::Duration::from_millis(100)).build();

    let options = SpawnOptions::builder().health_config(health_config).build();

    let actor_id = ActorId::of::<TestActor>("/health-test");
    let (mut actor_ctx, mut actor) =
        Actor::spawn(engine.clone(), actor_id.clone(), TestActor { count: 0 }, options).await?;

    // Start actor in background
    let actor_handle = tokio::spawn(async move {
        if let Err(e) = actor.start(&mut actor_ctx).await {
            error!("TestActor error: {}", e);
        }
    });

    // Create sender context
    let sender_id = ActorId::of::<TestActor>("/sender");
    let (sender_ctx, _) =
        Actor::spawn(engine.clone(), sender_id.clone(), TestActor { count: 0 }, SpawnOptions::default()).await?;

    // Check health immediately after spawn
    assert!(sender_ctx.check_actor_health(&actor_id).await?);

    // Wait a bit and check health again (should still be healthy due to updates)
    sleep(Duration::from_millis(300)).await;
    assert!(sender_ctx.check_actor_health(&actor_id).await?);

    // Kill actor and verify health reports as unhealthy
    actor_handle.abort();
    sleep(Duration::from_millis(200)).await;
    assert!(!sender_ctx.check_actor_health(&actor_id).await?);

    Ok(())
}

#[test(tokio::test)]
async fn test_health_monitoring_disabled() -> Result<(), TestError> {
    let engine = Engine::test().await?;

    // Create health config with monitoring disabled
    let health_config = HealthConfig::builder().update_interval(sql::Duration::from_millis(100)).build();

    let options = SpawnOptions::builder().health_config(health_config).build();

    let actor_id = ActorId::of::<TestActor>("/health-disabled");
    let (mut actor_ctx, mut actor) =
        Actor::spawn(engine.clone(), actor_id.clone(), TestActor { count: 0 }, options).await?;

    // Start actor in background
    let actor_handle = tokio::spawn(async move {
        if let Err(e) = actor.start(&mut actor_ctx).await {
            error!("TestActor error: {}", e);
        }
    });

    // Create sender context
    let sender_id = ActorId::of::<TestActor>("/sender");
    let (sender_ctx, _) =
        Actor::spawn(engine.clone(), sender_id.clone(), TestActor { count: 0 }, SpawnOptions::default()).await?;

    // Check health - should return true since monitoring is disabled
    assert!(sender_ctx.check_actor_health(&actor_id).await?);

    // Wait longer than default update interval
    sleep(Duration::from_secs(2)).await;

    // Should still return true since monitoring is disabled
    assert!(sender_ctx.check_actor_health(&actor_id).await?);

    actor_handle.abort();
    Ok(())
}

#[test(tokio::test)]
async fn test_health_check_before_send() -> Result<(), TestError> {
    let engine = Engine::test().await?;

    // Create health config with short update interval
    let health_config = HealthConfig::builder().update_interval(sql::Duration::from_millis(100)).build();

    let options = SpawnOptions::builder().health_config(health_config).build();

    let actor_id = ActorId::of::<TestActor>("/health-check");
    let (mut actor_ctx, mut actor) =
        Actor::spawn(engine.clone(), actor_id.clone(), TestActor { count: 0 }, options).await?;

    // Start actor in background
    let actor_handle = tokio::spawn(async move {
        if let Err(e) = actor.start(&mut actor_ctx).await {
            error!("TestActor error: {}", e);
        }
    });

    // Create sender context
    let sender_id = ActorId::of::<TestActor>("/sender");
    let (sender_ctx, _) =
        Actor::spawn(engine.clone(), sender_id.clone(), TestActor { count: 0 }, SpawnOptions::default()).await?;

    // Send message with health check enabled
    let send_options = SendOptions::builder().check_health(true).timeout(Duration::from_secs(1)).build();

    // First message should succeed
    let message = TestMessage { content: "Health check test".to_string() };
    let result = sender_ctx.send::<TestActor, TestMessage>(message.clone(), &actor_id, send_options.clone()).await;
    assert!(result.is_ok());

    // Kill the actor
    actor_handle.abort();
    sleep(Duration::from_millis(200)).await;

    // Message after kill should fail with UnhealthyActor error
    let result = sender_ctx.send::<TestActor, TestMessage>(message, &actor_id, send_options).await;

    match result {
        Err(SystemActorError::UnhealthyActor(_)) => (),
        _ => panic!("Expected UnhealthyActor error"),
    }

    Ok(())
}

#[test(tokio::test)]
async fn test_health_record_persistence() -> Result<(), TestError> {
    let engine = Engine::test().await?;

    let health_config = HealthConfig::builder().update_interval(sql::Duration::from_millis(100)).build();

    let options = SpawnOptions::builder().health_config(health_config.clone()).build();

    let actor_id = ActorId::of::<TestActor>("/health-persist");

    // First spawn
    let (mut actor_ctx, mut actor) =
        Actor::spawn(engine.clone(), actor_id.clone(), TestActor { count: 0 }, options.clone()).await?;

    let actor_handle = tokio::spawn(async move {
        if let Err(e) = actor.start(&mut actor_ctx).await {
            error!("TestActor error: {}", e);
        }
    });

    sleep(Duration::from_millis(200)).await;
    actor_handle.abort();

    // Respawn with same ID
    let (mut actor_ctx2, mut actor2) =
        Actor::spawn(engine.clone(), actor_id.clone(), TestActor { count: 0 }, options).await?;

    let actor_handle2 = tokio::spawn(async move {
        if let Err(e) = actor2.start(&mut actor_ctx2).await {
            error!("TestActor error: {}", e);
        }
    });

    // Create sender context
    let sender_id = ActorId::of::<TestActor>("/sender");
    let (sender_ctx, _) =
        Actor::spawn(engine.clone(), sender_id.clone(), TestActor { count: 0 }, SpawnOptions::default()).await?;

    // Health should be good after respawn
    assert!(sender_ctx.check_actor_health(&actor_id).await?);

    actor_handle2.abort();
    Ok(())
}
