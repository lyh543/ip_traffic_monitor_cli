// @generated automatically by Diesel CLI.

diesel::table! {
    ip_traffic (id) {
        id -> Integer,
        timestamp -> Text,
        remote_ip -> Text,
        tx_bytes -> Integer,
        pid -> Nullable<Integer>,
    }
}
