fn main() {
    unsafe {
        let ptr: *const i32 = std::ptr::null();
        std::process::exit(*ptr);
    }
}
