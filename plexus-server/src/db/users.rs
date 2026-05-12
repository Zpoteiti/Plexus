use serde::Serialize;
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub name: String,
    pub is_admin: bool,
    pub created_at: OffsetDateTime,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct UserWithPassword {
    pub id: Uuid,
    pub email: String,
    pub password_hash: String,
    pub name: String,
    pub is_admin: bool,
    pub created_at: OffsetDateTime,
}

pub async fn create_user(
    pool: &PgPool,
    email: &str,
    password_hash: &str,
    name: &str,
    is_admin: bool,
) -> Result<User, sqlx::Error> {
    sqlx::query_as::<_, User>(
        r#"
        INSERT INTO users (email, password_hash, name, is_admin)
        VALUES ($1, $2, $3, $4)
        RETURNING id, email, name, is_admin, created_at
        "#,
    )
    .bind(email)
    .bind(password_hash)
    .bind(name)
    .bind(is_admin)
    .fetch_one(pool)
    .await
}

pub async fn find_by_email(
    pool: &PgPool,
    email: &str,
) -> Result<Option<UserWithPassword>, sqlx::Error> {
    sqlx::query_as::<_, UserWithPassword>(
        r#"
        SELECT id, email, password_hash, name, is_admin, created_at
        FROM users
        WHERE email = $1
        "#,
    )
    .bind(email)
    .fetch_optional(pool)
    .await
}

pub async fn find_by_id(pool: &PgPool, id: Uuid) -> Result<Option<User>, sqlx::Error> {
    sqlx::query_as::<_, User>(
        r#"
        SELECT id, email, name, is_admin, created_at
        FROM users
        WHERE id = $1
        "#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await
}

pub async fn update_profile(
    pool: &PgPool,
    id: Uuid,
    email: Option<&str>,
    name: Option<&str>,
    password_hash: Option<&str>,
) -> Result<User, sqlx::Error> {
    sqlx::query_as::<_, User>(
        r#"
        UPDATE users
        SET
            email = COALESCE($2, email),
            name = COALESCE($3, name),
            password_hash = COALESCE($4, password_hash)
        WHERE id = $1
        RETURNING id, email, name, is_admin, created_at
        "#,
    )
    .bind(id)
    .bind(email)
    .bind(name)
    .bind(password_hash)
    .fetch_one(pool)
    .await
}

pub async fn delete_by_id(pool: &PgPool, id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
