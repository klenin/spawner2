fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.is_empty() {
        return;
    }

    let mut cmd = std::process::Command::new(args[0].as_str());
    for arg in args.iter().skip(1) {
        cmd.arg(arg.as_str());
    }

    let _ = cmd.spawn();
}
