use actix_web::cookie::Key;
use actix_web::dev::Server;
use actix_web::{App, HttpServer, web};
use actix_web_flash_messages::FlashMessagesFramework;
use actix_web_flash_messages::storage::CookieMessageStore;
use secrecy::{ExposeSecret, Secret};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use std::net::TcpListener;
use tracing_actix_web::TracingLogger;

use crate::configuration::{DatabaseSettings, Settings};
use crate::email_client::EmailClient;
use crate::routes::{
    confirm, health_check, home, login, login_form, publish_newsletter, subscribe,
};

pub fn get_connection_pool(db_configuration: &DatabaseSettings) -> PgPool {
    PgPoolOptions::new().connect_lazy_with(db_configuration.connect_options())
}

pub struct Application {
    port: u16,
    server: Server,
}

struct AppServerParams {
    listener: TcpListener,
    connection_pool: PgPool,
    email_client: EmailClient,
    base_url: String,
    hmac_secret: Secret<String>,
}

#[derive(Clone)]
pub struct HmacSecret(pub Secret<String>);

// We need to define a wrapper type in order to retrieve the URL
// in the `subscribe` handler.
// Retrieval from the context, in actix-web, is type-based: using
// a raw `String` would expose us to conflicts.
pub struct ApplicationBaseUrl(pub String);

impl Application {
    pub async fn build(configuration: Settings) -> Result<Self, std::io::Error> {
        let connection_pool = get_connection_pool(&configuration.database);
        let sender_email = configuration
            .email_client
            .sender()
            .expect("Invalid sender email address.");
        let timeout = configuration.email_client.timeout();
        let email_client = EmailClient::new(
            configuration.email_client.base_url,
            sender_email,
            configuration.email_client.authorization_token,
            timeout,
        );
        let address = format!(
            "{}:{}",
            configuration.application.host, configuration.application.port
        );
        let listener = TcpListener::bind(address)?;
        let port = listener.local_addr()?.port();
        let server = run(AppServerParams {
            listener,
            connection_pool,
            email_client,
            base_url: configuration.application.base_url,
            hmac_secret: configuration.application.hmac_secret,
        })?;

        Ok(Self { port, server })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub async fn run_until_stopped(self) -> Result<(), std::io::Error> {
        self.server.await
    }
}

// We need to mark `run` as public.
// It is no longer a binary entrypoint, therefore we can mark it as async
// without having to use any proc-macro incantation.
fn run(app_server_params: AppServerParams) -> Result<Server, std::io::Error> {
    tracing::debug!(
        "running app with email_client: {:?}",
        &app_server_params.email_client
    );
    // Wrap the pool using web::Data, which boils down to an Arc smart pointer
    let connection_pool = web::Data::new(app_server_params.connection_pool);
    let email_client = web::Data::new(app_server_params.email_client);
    let base_url = web::Data::new(ApplicationBaseUrl(app_server_params.base_url));
    let message_store = CookieMessageStore::builder(Key::from(
        app_server_params.hmac_secret.expose_secret().as_bytes(),
    ))
    .build();
    let message_framework = FlashMessagesFramework::builder(message_store).build();
    let hmac_secret = web::Data::new(HmacSecret(app_server_params.hmac_secret));
    // Capture `connection` from the surrounding environment
    let server = HttpServer::new(move || {
        App::new()
            // Middlewares are added using the `wrap` method on `App`
            .wrap(message_framework.clone())
            .wrap(TracingLogger::default())
            .route("/health_check", web::get().to(health_check))
            .route("/", web::get().to(home))
            .route("/login", web::get().to(login_form))
            .route("/login", web::post().to(login))
            .route("/newsletters", web::post().to(publish_newsletter))
            .route("/subscriptions", web::post().to(subscribe))
            .route("/subscriptions/confirm", web::get().to(confirm))
            // Get a pointer copy of the connection pool and attach it to the application state
            .app_data(connection_pool.clone())
            .app_data(email_client.clone())
            .app_data(base_url.clone())
            .app_data(hmac_secret.clone())
    })
    .listen(app_server_params.listener)?
    .run();
    Ok(server)
}
