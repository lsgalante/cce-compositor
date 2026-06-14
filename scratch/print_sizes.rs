fn main() {
    println!("wlr_output_state size: {}", std::mem::size_of::<cce::ffi::wlr_output_state>());
    println!("wlr_output_state alignment: {}", std::mem::align_of::<cce::ffi::wlr_output_state>());
}
