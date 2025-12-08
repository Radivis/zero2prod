use actix_web::dev::Server;
use actix_web::{App, HttpServer, web};
use sqlx::PgPool;
use std::net::TcpListener;
use tracing_actix_web::TracingLogger;

use crate::email_client::EmailClient;
use crate::routes::{health_check, subscribe};

// We need to mark `run` as public.
// It is no longer a binary entrypoint, therefore we can mark it as async
// without having to use any proc-macro incantation.
pub fn run(
    listener: TcpListener,
    connection_pool: PgPool,
    email_client: EmailClient,
) -> Result<Server, std::io::Error> {
    // Wrap the pool using web::Data, which boils down to an Arc smart pointer
    let connection_pool = web::Data::new(connection_pool);
    let email_client = web::Data::new(email_client);
    // Capture `connection` from the surrounding environment
    let server = HttpServer::new(move || {
        App::new()
            // Middlewares are added using the `wrap` method on `App`
            .wrap(TracingLogger::default())
            .route("/health_check", web::get().to(health_check))
            // A new entry in our routing table for POST /subscriptions requests
            .route("/subscriptions", web::post().to(subscribe))
            // Get a pointer copy of the connection pool and attach it to the application state
            .app_data(connection_pool.clone())
    })
    .listen(listener)?
    .run();
    Ok(server)
}
