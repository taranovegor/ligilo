use sqlx::PgPool;

pub async fn get_url(pool: &PgPool, code: &str) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar::<_, String>("SELECT url FROM urls WHERE code = $1")
        .bind(code)
        .fetch_optional(pool)
        .await
}

pub async fn create_url(pool: &PgPool, code: &str, url: &str) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO urls (code, url) VALUES ($1, $2)")
        .bind(code)
        .bind(url)
        .execute(pool)
        .await?;
    Ok(())
}
