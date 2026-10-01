fn main() {
    let reports = polymarket_trading_control_plane::demo::run_all().expect("demo scenarios");
    polymarket_trading_control_plane::demo::print_reports(&reports);
}
