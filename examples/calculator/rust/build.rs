fn main() {
    winrt_component_build::WinrtComponentBindgen::new()
        .rdl("calculator.rdl", "target/winmd/CalculatorDemo.winmd")
        .output("src/bindings.rs")
        .filter("CalculatorDemo")
        .implement("CalculatorDemo")
        .generate();
}
