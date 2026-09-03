struct Provider {
    config: omnia_test::guest::MapConfig,
}

omnia_test::forward!(impl Provider { Telemetry => config });

fn main() {}
