fn main() {
    let args: Vec<String> = std::env::args().collect();
    cce_fx::run_cce_ctl(args);
}
