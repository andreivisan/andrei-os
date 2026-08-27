pub mod handlers;
pub mod models;
pub mod repository;

/*
REPOSITORY
*/
pub type RepositoryResult<T> = Result<T, sqlx::Error>;
