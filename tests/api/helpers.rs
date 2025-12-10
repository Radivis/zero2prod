use once_cell::sync::Lazy;
use secrecy::Secret;
use sqlx::{Connection, Executor, PgConnection, PgPool};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;
use tracing_appender::non_blocking::WorkerGuard;
use uuid::Uuid;
use wiremock::MockServer;

use zero2prod::configuration::{DatabaseSettings, get_configuration};
use zero2prod::startup::Application;
use zero2prod::telemetry::init_subscriber;

// This holds the guard for the entire lifetime of the test process
static LOG_GUARD: Lazy<Mutex<Option<WorkerGuard>>> = Lazy::new(|| Mutex::new(None));

// Ensure that the `tracing` stack is only initialised once using `LazyLock`

static TRACING: LazyLock<()> = LazyLock::new(|| {
    /*let default_filter_level = "info".to_string();
    let subscriber_name = "test".to_string();
    // We cannot assign the output of `get_subscriber` to a variable based on the
    // value TEST_LOG` because the sink is part of the type returned by
    // `get_subscriber`, therefore they are not the same type. We could work around
    // it, but this is the most straight-forward way of moving forward.
    if std::env::var("TEST_LOG").is_ok() {
        let subscriber = get_subscriber(subscriber_name, default_filter_level, std::io::stdout);
        init_subscriber(subscriber);
    } else {
        let subscriber = get_subscriber(subscriber_name, default_filter_level, std::io::sink);
        init_subscriber(subscriber);
    }*/
    let file_appender = tracing_appender::rolling::never("tests/logs", "integration.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    // Store the guard so it lives forever
    let mut guard_slot = LOG_GUARD.lock().unwrap();
    *guard_slot = Some(guard);

    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(non_blocking)
        .with_ansi(false)
        .with_target(true)
        .with_line_number(true)
        .with_thread_names(true)
        .with_file(true)
        .with_env_filter("debug")
        .init();
});

#[derive(Debug)]
pub struct TestApp {
    pub address: String,
    pub connection_pool: PgPool,
    pub email_server: MockServer,
}

impl TestApp {
    pub async fn post_subscriptions(&self, body: String) -> reqwest::Response {
        reqwest::Client::new()
            .post(format!("{}/subscriptions", &self.address))
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(body)
            .send()
            .await
            .expect("Failed to execute request.")
    }
}

#[tracing::instrument(name = "Spawning test application")]
pub async fn spawn_app() -> TestApp {
    // The first time `initialize` is invoked the code in `TRACING` is executed.
    // All other invocations will instead skip execution.
    LazyLock::force(&TRACING);

    // Launch a mock server to stand in for Postmark's API
    let email_server = MockServer::start().await;

    // Randomise configuration to ensure test isolation
    let configuration = {
        let mut c = get_configuration().expect("Failed to read configuration.");
        // Use a different database for each test case
        c.database.database_name = Uuid::new_v4().to_string();
        // Use a random OS port
        c.application.port = 0;
        c.email_client.base_url = email_server.uri();
        c
    };

    let connection_pool = configure_database(&configuration.database).await;

    // Notice the .clone!
    let application = Application::build(configuration.clone())
        .await
        .expect("Failed to build application.");
    let address = format!("http://127.0.0.1:{}", application.port());

    #[allow(clippy::let_underscore_future)]
    let _ = tokio::spawn(application.run_until_stopped());

    let test_app = TestApp {
        address,
        connection_pool,
        email_server,
    };

    tracing::debug!("test_app spawned with details: {:?}", &test_app);

    test_app
}

pub async fn configure_database(config: &DatabaseSettings) -> PgPool {
    // Create database
    let maintenance_settings = DatabaseSettings {
        database_name: "postgres".to_string(),
        username: "postgres".to_string(),
        password: Secret::new("STOPSERG!2345already".to_string()),
        ..config.clone()
    };
    let mut connection = PgConnection::connect_with(&maintenance_settings.connect_options())
        .await
        .expect("Failed to connect to Postgres");
    connection
        .execute(format!(r#"CREATE DATABASE "{}";"#, config.database_name).as_str())
        .await
        .expect("Failed to create database.");
    // Migrate database
    let connection_pool = PgPool::connect_with(config.connect_options())
        .await
        .expect("Failed to connect to Postgres.");
    sqlx::migrate!("./migrations")
        .run(&connection_pool)
        .await
        .expect("Failed to migrate the database");
    connection_pool
}

pub async fn retry<F, Fut, T>(mut f: F, max_retries: u8) -> T
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, sqlx::Error>>,
{
    for attempt in 0..max_retries {
        match f().await {
            Ok(value) => return value,
            Err(sqlx::Error::RowNotFound) if attempt < max_retries - 1 => {
                tokio::time::sleep(Duration::from_millis(100)).await; // 100ms backoff
            }
            Err(e) => panic!("{1}: {:?}", e, "Non-RowNotFound error"),
        }
    }
    f().await.expect("Retry exhausted")
}
