use std::fs::File;
use std::io::Write;
use std::path::Path;

fn main()
{
    #[cfg(target_os = "linux")]
    {
        let out_dir = std::env::var("OUT_DIR").expect("Output directory unavailable.");
        let run = escargot::CargoBuild::new()
            .current_release()
            .current_target()
            .manifest_path("../rstack-child/Cargo.toml")
            .target_dir(&out_dir)
            .run()
            .expect("Compiling rstack-child failed.");

        let child_template = r#"
        fn launch_child() -> rstack_self::Result<rstack_self::Trace> {
            let exe = "PATH";
            Ok(rstack_self::trace(&mut Command::new(exe))?)
        }
        "#;
        let child_template = child_template.replace(
            "PATH",
            run.path().to_str().expect("Unexpected characters in path."),
        );

        let dest_path = Path::new(&out_dir).join("child.rs");
        let mut f = File::create(&dest_path).expect("Opening child.rs failed.");
        f.write_all(child_template.as_bytes())
            .expect("Writing child.rs failed.");
    }

    println!("cargo:rerun-if-changed=build.rs");
}
