use crate::domain::Alert;

pub fn unresolved_mut<'a>(alerts: &'a mut [Alert], key: &str) -> Option<&'a mut Alert> {
    alerts
        .iter_mut()
        .find(|alert| !alert.resolved && alert.dedup_key == key)
}
