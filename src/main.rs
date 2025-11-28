use zero2prod::run;
use std::net::TcpListener;
#[tokio::main]
async fn main() -> Result<(), std::io::Error> {
    run(TcpListener::bind("127.0.0.1:8000").expect("Failed to bind to port 8000"))?.await
}