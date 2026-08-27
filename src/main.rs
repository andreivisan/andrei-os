pub mod application;
pub mod infrastructure;
pub mod modules;

#[tokio::main]
async fn main() {
    application::run().await;
}
