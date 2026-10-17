# SPDX-License-Identifier: LGPL-2.1-or-later
"""Builds the shared library, compiles the C++ interface's test with a given compiler, and runs it.

    python3 crates/bil-cpp/tests/run_cpp.py gcc|gcc-asan|clang|msvc [cargo arguments...]

GCC and Clang build with -std=c++20 and every warning an error; MSVC with /std:c++20 /W4 /WX,
from the newest Visual Studio that vswhere finds. gcc-asan adds AddressSanitizer, with leak
checks, and UndefinedBehaviorSanitizer. Exits non-zero if anything fails.
"""
import os
import pathlib
import platform
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve().parent
CRATE = HERE.parent
ROOT = CRATE.parent.parent
TARGET = ROOT / "target" / "debug"
OUT = ROOT / "target" / "cpp-test"


def run(cmd, **kw):
    print("+", " ".join(str(c) for c in cmd), flush=True)
    subprocess.run(cmd, check=True, **kw)


def main() -> None:
    if len(sys.argv) < 2 or sys.argv[1] not in ("gcc", "gcc-asan", "clang", "msvc"):
        sys.exit(__doc__)
    compiler, cargo_args = sys.argv[1], sys.argv[2:]
    run(["cargo", "build", "-p", "bil-cpp", *cargo_args], cwd=ROOT)
    OUT.mkdir(parents=True, exist_ok=True)
    test = HERE / "cpp" / "test.cpp"
    include = CRATE / "include"
    env = dict(os.environ)
    if compiler == "msvc":
        exe = OUT / "bil_cpp_test_msvc.exe"
        vswhere = pathlib.Path(os.environ["ProgramFiles(x86)"]) / "Microsoft Visual Studio/Installer/vswhere.exe"
        vs = subprocess.run(
            [str(vswhere), "-latest", "-products", "*", "-property", "installationPath"],
            check=True, capture_output=True, text=True,
        ).stdout.strip()
        vcvars = pathlib.Path(vs) / "VC/Auxiliary/Build/vcvars64.bat"
        # cmd's quoting is its own, so the commands go in a batch file.
        bat = OUT / "build.bat"
        bat.write_text(
            f'@call "{vcvars}" >nul\r\n'
            f'cl /nologo /std:c++20 /EHsc /W4 /WX /permissive- /I "{include}" "{test}" '
            f'/Fo"{OUT / "test.obj"}" /Fe"{exe}" /link "{TARGET / "bil.dll.lib"}"\r\n'
        )
        run(["cmd", "/c", str(bat)])
        env["PATH"] = f"{TARGET}{os.pathsep}{env['PATH']}"
    else:
        cxx = "clang++" if compiler == "clang" else "g++"
        exe = OUT / f"bil_cpp_test_{compiler}"
        flags = ["-std=c++20", "-Wall", "-Wextra", "-Wpedantic", "-Werror", "-pthread"]
        if compiler == "gcc-asan":
            flags += ["-g", "-fsanitize=address,undefined", "-fno-sanitize-recover=all"]
            env["ASAN_OPTIONS"] = "detect_leaks=1"
        run([cxx, *flags, "-I", str(include), str(test), "-o", str(exe),
             "-L", str(TARGET), "-lbil", f"-Wl,-rpath,{TARGET}"])
        key = "DYLD_LIBRARY_PATH" if platform.system() == "Darwin" else "LD_LIBRARY_PATH"
        env[key] = f"{TARGET}{os.pathsep}{env.get(key, '')}"
    run([str(exe)], env=env)


if __name__ == "__main__":
    main()
