struct Provider {
    config: omnia_test::guest::MapConfig,
}

omnia_test::delegate!(impl Provider { Telemetry => config });

fn main() {}
