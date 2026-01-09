use std::env;

guest_macro::guest!({
    owner: "at",
    http: [
        "/jobs/detector": get(DetectionRequest with_query, DetectionReply),
        "/god-mode/set-trip/{vehicle_id}/{trip_id}": post(SetTripRequest with_body, SetTripReply),
    ],
    messaging: [
        format!("{env}-realtime-r9k.v1", env = env::var("ENV").unwrap_or("dev".to_string())): R9kMessage,
    ],
    capabilities: [
        HttpRequest,
        Identity,
        Publisher,
        StateStore
    ],
    environment: [
        ENV: String = "dev",
        BLOCK_MGT_URL: String,
        CC_STATIC_URL: String,
        FLEET_URL: String,
        GTFS_STATIC_URL: String ,
    ]
});
