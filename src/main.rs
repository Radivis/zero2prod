use sqlx::PgPool;
use std::net::TcpListener;
use zero2prod::configuration::get_configuration;
use zero2prod::startup::run;
use zero2prod::telemetry::{get_subscriber, init_subscriber};

#[tokio::main]
async fn main() -> Result<(), std::io::Error> {
    let subscriber = get_subscriber("zero2prod".into(), "info".into(), std::io::stdout);
    init_subscriber(subscriber);

    // Panic if we can't read configuration
    let configuration = get_configuration().expect("Failed to read configuration.");
    let connection_pool = PgPool::connect_lazy_with(configuration.database.connect_options());
    let host = configuration.application.host;
    let address = format!("{}:{}", host, configuration.application.port);
    let listener = TcpListener::bind(address)?;
    let post_binding_error_message =
        format!("Failed to bind to port {}", configuration.application.port);
    run(listener, connection_pool)
        .expect(&post_binding_error_message)
        .await
}
