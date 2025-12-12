use once_cell::sync::Lazy;
use secrecy::Secret;
use sqlx::{Connection, Executor, PgConnection, PgPool};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;
use tracing_appender::non_blocking::{NonBlocking, WorkerGuard};
use uuid::Uuid;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate, Times};
use zero2prod::configuration::{DatabaseSettings, get_configuration};
use zero2prod::startup::Application;
use zero2prod::telemetry::{get_subscriber, init_subscriber};

// This holds the guard for the entire lifetime of the test process
static LOG_GUARD: Lazy<Mutex<Option<WorkerGuard>>> = Lazy::new(|| Mutex::new(None));

// Ensure that the `tracing` stack is only initialised once using `LazyLock`

static TRACING: LazyLock<()> = LazyLock::new(|| {
    let default_filter_level = "info".to_string();
    let subscriber_name = "test".to_string();
    let loglevel = std::env::var("LOGLEVEL").unwrap_or(default_filter_level);
    // We cannot assign the output of `get_subscriber` to a variable based on the
    // value TEST_LOG` because the sink is part of the type returned by
    // `get_subscriber`, therefore they are not the same type. We could work around
    // it, but this is the most straight-forward way of moving forward.
    if std::env::var("TEST_LOG").is_ok() {
        let subscriber = get_subscriber(subscriber_name, loglevel, std::io::stdout);
        init_subscriber(subscriber);
    } else {
        let subscriber = get_subscriber(subscriber_name, loglevel, test_writer());
        init_subscriber(subscriber);
    }
});

pub fn test_writer() -> NonBlocking {
    // nextest passes the test name via --exact argument
    // uncomment the next line and run "cargo test --no-capture" to see how the args are structured:
    // std::env::args().for_each(|arg| println!("Environment argument: {}", arg));
    let test_name = std::env::args()
        .skip_while(|arg| arg != "--exact")
        .nth(1) // we need the first arg exactly after "--exact"
        .and_then(|arg| arg.split("::").last().map(str::to_owned))
        .unwrap_or("unlabeled_test".into())
        .replace(' ', "_");

    let _ = std::fs::create_dir_all("tests/logs");
    let _ = std::fs::create_dir_all("tests/logs/nextest");
    let _ = std::fs::create_dir_all("tests/logs/cargo_test");
    let filename = if test_name != "unlabeled_test" {
        format!("tests/logs/nextest/{}.log", test_name)
    } else {
        "tests/logs/cargo_test/integration.log".into()
    };
    let file = std::fs::File::create(filename).expect("Failed to create log file");
    let (non_blocking, guard) = tracing_appender::non_blocking(file);

    // Store guard so it lives until process end
    let mut slot = LOG_GUARD.lock().unwrap();
    *slot = Some(guard);

    non_blocking
}

/// Confirmation links embedded in the request to the email API.
pub struct ConfirmationLinks {
    pub html: reqwest::Url,
    pub plain_text: reqwest::Url,
}

#[derive(Debug)]
pub struct TestApp {
    pub address: String,
    pub port: u16,
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

    /// Extract the confirmation links embedded in the request to the email API.
    pub fn get_confirmation_links(&self, email_request: &wiremock::Request) -> ConfirmationLinks {
        let body: serde_json::Value = serde_json::from_slice(&email_request.body).unwrap();
        // Extract the link from one of the request fields.
        let get_link = |s: &str| {
            let links: Vec<_> = linkify::LinkFinder::new()
                .links(s)
                .filter(|l| *l.kind() == linkify::LinkKind::Url)
                .collect();
            assert_eq!(links.len(), 1);
            let raw_link = links[0].as_str().to_owned();
            let mut confirmation_link = reqwest::Url::parse(&raw_link).unwrap();
            // Let's make sure we don't call random APIs on the web
            assert_eq!(confirmation_link.host_str().unwrap(), "127.0.0.1");
            confirmation_link.set_port(Some(self.port)).unwrap();
            confirmation_link
        };

        let html = get_link(body["HtmlBody"].as_str().unwrap());
        let plain_text = get_link(body["TextBody"].as_str().unwrap());
        ConfirmationLinks { html, plain_text }
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
    let port = application.port();

    #[allow(clippy::let_underscore_future)]
    let _ = tokio::spawn(application.run_until_stopped());

    let test_app = TestApp {
        address,
        port,
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

pub async fn mount_mock_email_server(
    email_server: &MockServer,
    request_num_expectation: Option<Times>,
) -> () {
    let mock = Mock::given(path("/email"))
        .and(method("POST"))
        .respond_with(ResponseTemplate::new(200));
    if let Some(num_expected_requests) = request_num_expectation {
        mock.expect(num_expected_requests).mount(email_server).await
    } else {
        mock.mount(email_server).await
    }
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
