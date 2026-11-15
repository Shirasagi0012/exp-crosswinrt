fn main() {
    winrt_component_build::WinrtComponentBindgen::new()
        .rdl("async_demo.rdl", "target/winmd/AsyncDemo.winmd")
        .output("src/bindings.rs")
        .filter("AsyncDemo")
        .implement("AsyncDemo")
        .generate();
}
