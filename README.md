# exp-crosswinrt

A mostly vibe-coded experiment running WinRT components written in **Rust** and
consumed from **.NET 10 / C#** (via **CsWinRT 3.0**) on **Linux** (and probably
macOS), using unmodified upstream `windows-rs` and `cswinrt`.

In other words, it is entirely possible—and surprisingly straightforward, since
the CsWinRT 3.0 runtime no longer requires the Windows SDK to compile and run—to
use the WinRT ABI for cross-language FFI on operating systems other than
Windows! Performance-wise, it's actually quite fast, outperforming UniFFI (with
`UniFFISharp` as the C# generator) in most cases.

## Repository Structure

```text
exp-crosswinrt/
├── crates/
│   ├── winrt-cross-pal/           # Cross-platform WinRT PAL (HSTRING, CoTaskMem, MTA, activation dispatch)
│   ├── winrt-server-shims/        # IActivationFactory / IInspectable / ComArray helpers for Linux
│   ├── winrt-future/              # Cross-platform IAsyncAction / IAsyncOperation<T> backed by Tokio
│   └── winrt-component-build/     # Build-time RDL -> WinMD -> windows-bindgen + factory export codegen
├── dotnet/
│   ├── WinRT.CrossHosting/        # .NET host registration & PAL hooks for WinRT.Runtime on Linux
│   └── WinRT.RustComponent.Build/ # MSBuild props/targets for Contract, Projection, Interop & Cargo build
├── examples/
│   ├── calculator/                # End-to-end Rust <-> C# WinRT component example
│   │   ├── rust/                  # RDL contract + Rust #[implement] component + build.rs
│   │   ├── projection/            # CsWinRT 3.0 reference projection + forwarder assembly
│   │   └── app/                   # .NET 10 console host exercising static/instance/CCW/array/error ABI
│   └── async_demo/                # Bidirectional async interop (C# Task/await <-> Rust Tokio Futures)
│       ├── rust/                  # RDL async contract + Tokio-driven IAsyncAction/IAsyncOperation impls
│       ├── projection/            # CsWinRT 3.0 reference projection + forwarder assembly
│       └── app/                   # .NET 10 console host testing await, cancel, progress, tokio::join!, errors
├── scripts/
│   └── build-cswinrt.sh           # Builds required CsWinRT 3.0 tools, runtime, and SDK projections on Linux
└── third_party/
    ├── windows-rs/                # Unmodified submodule (https://github.com/microsoft/windows-rs.git)
    └── cswinrt/                   # Unmodified submodule (https://github.com/microsoft/cswinrt)
```

## Quick Start

### 1. Clone and initialize submodules

```bash
git clone --recursive https://github.com/Shirasagi0012/exp-crosswinrt
cd exp-crosswinrt
```

### 2. Build CsWinRT 3.0 artifacts from `third_party/cswinrt`

```bash
./scripts/build-cswinrt.sh
```

This builds `WinRT.Runtime` (`Release`, `Debug`, and `net10.0-ref`), CsWinRT 3.0 generator tools (`cswinrtwinmdgen`, `cswinrtprojectionrefgen`, `cswinrtimplgen`, `cswinrtprojectiongen`, `cswinrtinteropgen`, `WinRT.Generator.Tasks`, `WinRT.SourceGenerator`), `WindowsRuntime.Internal.winmd`, `WinRT.Sdk.Projection`, and `Microsoft.Windows.SDK.NET` without modifying `third_party/cswinrt`.

### 3. Run the examples

Calculator (sync ABI):

```bash
dotnet run -c Release --project examples/calculator/app/Calculator.App.csproj
```

Async Demo (C# `Task`/`await` <-> Rust `tokio` via `IAsyncAction` / `IAsyncOperation<T>`):

```bash
dotnet run -c Release --project examples/async_demo/app/AsyncDemo.App.csproj
```

MSBuild automatically drives:
1. `cargo build` + `build.rs` (`calculator.rdl` -> `CalculatorDemo.winmd` + `src/bindings.rs` + `server_gen.rs` + `libcalculator_rust.so`)
2. `Calculator.Projection` (`CalculatorDemo.winmd` -> CsWinRT 3.0 ref/forwarder assemblies)
3. `Calculator.App` (merged `WinRT.Projection.dll` + `WinRT.Interop.dll` + native PAL registration + execution)
