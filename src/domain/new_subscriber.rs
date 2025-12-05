use crate::domain::subscriber_email_address::SubscriberEmailAddress;
use crate::domain::subscriber_name::SubscriberName;

pub struct NewSubscriber {
    pub email: SubscriberEmailAddress,
    pub name: SubscriberName,
}
