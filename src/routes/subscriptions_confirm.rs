use actix_web::{HttpResponse, web};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(serde::Deserialize)]
#[allow(unused)]
pub struct Parameters {
    subscription_token: String,
}
#[tracing::instrument(
    name = "Confirm a pending subscriber",
    skip(parameters, db_connection_pool)
)]
pub async fn confirm(
    parameters: web::Query<Parameters>,
    db_connection_pool: web::Data<PgPool>,
) -> HttpResponse {
    let subscriber_id =
        match get_subscriber_id_from_token(&db_connection_pool, &parameters.subscription_token)
            .await
        {
            Ok(subscriber_id) => subscriber_id,
            Err(_) => return HttpResponse::InternalServerError().finish(),
        };
    match subscriber_id {
        // Non-existing token!
        None => HttpResponse::Unauthorized().finish(),
        Some(subscriber_id_) => {
            if confirm_subscriber(&db_connection_pool, subscriber_id_)
                .await
                .is_err()
            {
                return HttpResponse::InternalServerError().finish();
            }
            HttpResponse::Ok().finish()
        }
    }
}

#[tracing::instrument(
    name = "Mark subscriber as confirmed",
    skip(subscriber_id, db_connection_pool)
)]
pub async fn confirm_subscriber(
    db_connection_pool: &PgPool,
    subscriber_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        r#"UPDATE subscriptions SET status = 'confirmed' WHERE id = $1"#,
        subscriber_id,
    )
    .execute(db_connection_pool)
    .await
    .map_err(|e| {
        tracing::error!("Failed to execute query: {:?}", e);
        e
    })?;
    Ok(())
}

#[tracing::instrument(
    name = "Get subscriber_id from token",
    skip(subscription_token, db_connection_pool)
)]
pub async fn get_subscriber_id_from_token(
    db_connection_pool: &PgPool,
    subscription_token: &str,
) -> Result<Option<Uuid>, sqlx::Error> {
    let result = sqlx::query!(
        "SELECT subscriber_id FROM subscription_tokens \
        WHERE subscription_token = $1",
        subscription_token,
    )
    .fetch_optional(db_connection_pool)
    .await
    .map_err(|e| {
        tracing::error!("Failed to execute query: {:?}", e);
        e
    })?;
    Ok(result.map(|r| r.subscriber_id))
}
