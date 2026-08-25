pub mod handlers;
pub mod models;
pub mod repository;
pub mod router;

/*
REPOSITORY
*/

pub type RepositoryResult<T> = Result<T, sqlx::Error>;
