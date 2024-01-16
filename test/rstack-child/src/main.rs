fn main() {
    #[cfg(target_os = "linux")]
    {
        let err = rstack_self::child();
        eprintln!("{:?}", err);
        err.expect("Capturing callstack with rstack-self failed.");
    }

    #[cfg(not(target_os = "linux"))]
    {
        panic!("Unsupported operating system.");
    }
}
