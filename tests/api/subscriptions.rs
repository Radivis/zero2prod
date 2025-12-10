use crate::helpers::{retry, spawn_app};
use wiremock::matchers::{method, path};
use wiremock::{Mock, ResponseTemplate};

#[tokio::test]
#[tracing::instrument(name = "subscribe_returns_a_200_for_valid_form_data")]
async fn subscribe_returns_a_200_for_valid_form_data() {
    // Arrange
    let test_app = spawn_app().await;
    tracing::debug!(
        "test_app started with email_server: {:?}",
        &test_app.email_server
    );

    Mock::given(path("/email"))
        .and(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&test_app.email_server)
        .await;
    // Act
    let body = "name=le%20guin&email=ursula_le_guin%40gmail.com";
    let response = test_app.post_subscriptions(body.into()).await;

    // Assert
    assert_eq!(200, response.status().as_u16());

    let saved = retry(
        || async {
            sqlx::query!("SELECT email, name FROM subscriptions",)
                .fetch_one(&test_app.connection_pool)
                .await
        },
        5,
    )
    .await;

    assert_eq!(saved.email, "ursula_le_guin@gmail.com");
    assert_eq!(saved.name, "le guin");
}

#[tokio::test]
#[tracing::instrument(name = "subscribe_returns_a_400_when_data_is_missing")]
async fn subscribe_returns_a_400_when_data_is_missing() {
    // Arrange
    let test_app = spawn_app().await;
    tracing::debug!(
        "test_app started with email_server: {:?}",
        &test_app.email_server
    );
    let test_cases = vec![
        ("name=le%20guin", "missing the email"),
        ("email=ursula_le_guin%40gmail.com", "missing the name"),
        ("", "missing both name and email"),
    ];
    for (invalid_body, error_message) in test_cases {
        // Act
        let response = test_app.post_subscriptions(invalid_body.into()).await;

        // Assert
        assert_eq!(
            400,
            response.status().as_u16(),
            // Additional customised error message on test failure
            "The API did not fail with 400 Bad Request when the payload was {}.",
            error_message
        );
    }
}

#[tokio::test]
#[tracing::instrument(name = "subscribe_returns_a_400_when_fields_are_present_but_invalid")]
async fn subscribe_returns_a_400_when_fields_are_present_but_invalid() {
    // Arrange
    let test_app = spawn_app().await;
    tracing::debug!(
        "test_app started with email_server: {:?}",
        &test_app.email_server
    );
    let test_cases = vec![
        ("name=&email=ursula_le_guin%40gmail.com", "empty name"),
        ("name=Ursula&email=", "empty email"),
        ("name=Ursula&email=definitely-not-an-email", "invalid email"),
    ];
    for (invalid_body, error_message) in test_cases {
        // Act
        let response = test_app.post_subscriptions(invalid_body.into()).await;

        // Assert
        assert_eq!(
            400,
            response.status().as_u16(),
            // Additional customised error message on test failure
            "The API did not return a 400 Bad Request when the payload was {}.",
            error_message
        );
    }
}

#[tokio::test]
#[tracing::instrument(name = "subscribe_sends_a_confirmation_email_for_valid_data")]
async fn subscribe_sends_a_confirmation_email_for_valid_data() {
    // Arrange
    let test_app = spawn_app().await;
    tracing::debug!(
        "test_app started with email_server: {:?}",
        &test_app.email_server
    );
    let body = "name=le%20guin&email=ursula_le_guin%40gmail.com";
    Mock::given(path("/email"))
        .and(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&test_app.email_server)
        .await;
    // Act
    test_app.post_subscriptions(body.into()).await;
    // Assert
    // Mock
}
