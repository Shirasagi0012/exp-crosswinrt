#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CSWINRT_SRC="${ROOT_DIR}/third_party/cswinrt/src"
CONFIG="${CSWINRT_CONFIG:-Debug}"

if [[ ! -d "${CSWINRT_SRC}" ]]; then
  echo "error: ${CSWINRT_SRC} not found. Run: git submodule update --init --recursive" >&2
  exit 1
fi

# CsWinRT's Directory.Build.targets checks for src/TestWinRT even when building tool projects only.
# `TestWinRT` is listed in third_party/cswinrt/.gitignore, so creating an empty marker dir keeps the submodule clean.
mkdir -p "${CSWINRT_SRC}/TestWinRT"

# - Clear PkgMicrosoft_DiaSymReader_Pdb2Pdb on Linux so CopyEmbeddedSymbols does not invoke Windows Pdb2Pdb.exe.
# - Disable PublishAot for generator tools so they compile as managed net10.0 apphosts without NativeAOT packs.
# - Enable EnableWindowsTargeting so WinRT.Internal / SDK projections can build metadata on Linux.
MSBUILD_ARGS=(-p:PkgMicrosoft_DiaSymReader_Pdb2Pdb= -p:PublishAot=false -p:EnableWindowsTargeting=true)

dotnet build "${CSWINRT_SRC}/WinRT.Runtime/WinRT.Runtime.csproj" -c Release "${MSBUILD_ARGS[@]}"
dotnet build "${CSWINRT_SRC}/WinRT.Runtime/WinRT.Runtime.csproj" -c "${CONFIG}" "${MSBUILD_ARGS[@]}"
dotnet build "${CSWINRT_SRC}/WinRT.Runtime/WinRT.Runtime.csproj" -c "${CONFIG}" \
  -p:CsWinRTBuildReferenceAssembly=true \
  -p:OutputPath="${CSWINRT_SRC}/WinRT.Runtime/bin/${CONFIG}/net10.0-ref/" \
  -p:IntermediateOutputPath="${CSWINRT_SRC}/WinRT.Runtime/obj/${CONFIG}/net10.0-ref/" \
  "${MSBUILD_ARGS[@]}"

for proj in \
  WinRT.Generator.Tasks/WinRT.Generator.Tasks.csproj \
  WinRT.WinMD.Generator/WinRT.WinMD.Generator.csproj \
  WinRT.Projection.Ref.Generator/WinRT.Projection.Ref.Generator.csproj \
  WinRT.Impl.Generator/WinRT.Impl.Generator.csproj \
  WinRT.Projection.Generator/WinRT.Projection.Generator.csproj \
  WinRT.Interop.Generator/WinRT.Interop.Generator.csproj \
  WinRT.SourceGenerator/WinRT.SourceGenerator.csproj
do
  dotnet build "${CSWINRT_SRC}/${proj}" -c "${CONFIG}" "${MSBUILD_ARGS[@]}"
done

# CsWinRT's MSBuild tasks / WinRT.Internal invoke `<tool>.exe` by name; symlink the Linux apphosts in `bin/`.
for pair in \
  "WinRT.WinMD.Generator:cswinrtwinmdgen" \
  "WinRT.Projection.Ref.Generator:cswinrtprojectionrefgen" \
  "WinRT.Impl.Generator:cswinrtimplgen" \
  "WinRT.Projection.Generator:cswinrtprojectiongen" \
  "WinRT.Interop.Generator:cswinrtinteropgen"
do
  dir="${CSWINRT_SRC}/${pair%%:*}/bin/${CONFIG}/net10.0"
  tool="${pair##*:}"
  if [[ -f "${dir}/${tool}" && ! -e "${dir}/${tool}.exe" ]]; then
    ln -s "${tool}" "${dir}/${tool}.exe"
  fi
done

SDK_PKG_VERSION="${CSWINRT_SDK_PACKAGE_VERSION:-10.0.26100.7705}"
SDK_BUILD="${CSWINRT_SDK_BUILD:-26100}"

dotnet build "${CSWINRT_SRC}/WinRT.Internal/WinRT.Internal.csproj" -c "${CONFIG}" "${MSBUILD_ARGS[@]}"
dotnet build "${CSWINRT_SRC}/WinRT.Sdk.Projection/WinRT.Sdk.Projection.csproj" -c "${CONFIG}" -p:SdkPackageVersion="${SDK_PKG_VERSION}" -p:WindowsSdkBuild="${SDK_BUILD}" -p:_SdkPackageName=microsoft.windows.sdk.contracts "${MSBUILD_ARGS[@]}"
dotnet build "${CSWINRT_SRC}/Projections/Windows/Windows.csproj" -c "${CONFIG}" -f net10.0 -p:GenerateTestProjection=true -p:CsWinRTWindowsMetadata="${CSWINRT_SRC}/WinRT.Sdk.Projection/obj/${CONFIG}/${SDK_BUILD}/sdkwinmds" "${MSBUILD_ARGS[@]}"
