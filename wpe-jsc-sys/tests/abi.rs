// This file was generated by gir (https://github.com/gtk-rs/gir)
// from /usr/share/gir-1.0
// from ../gir-files
// DO NOT EDIT

#![cfg(unix)]

use std::error::Error;
use std::ffi::OsString;
use std::mem::{align_of, size_of};
use std::path::Path;
use std::process::{Command, Stdio};
use std::{env, str};

use tempfile::Builder;
use wpe_jsc_sys::*;

static PACKAGES: &[&str] = &["wpe-webkit-2.0"];

#[derive(Clone, Debug)]
struct Compiler {
    pub args: Vec<String>,
}

impl Compiler {
    pub fn new() -> Result<Self, Box<dyn Error>> {
        let mut args = get_var("CC", "cc")?;
        args.push("-Wno-deprecated-declarations".to_owned());
        // For _Generic
        args.push("-std=c11".to_owned());
        // For %z support in printf when using MinGW.
        args.push("-D__USE_MINGW_ANSI_STDIO".to_owned());
        args.extend(get_var("CFLAGS", "")?);
        args.extend(get_var("CPPFLAGS", "")?);
        args.extend(pkg_config_cflags(PACKAGES)?);
        Ok(Self { args })
    }

    pub fn compile(&self, src: &Path, out: &Path) -> Result<(), Box<dyn Error>> {
        let mut cmd = self.to_command();
        cmd.arg(src);
        cmd.arg("-o");
        cmd.arg(out);
        let status = cmd.spawn()?.wait()?;
        if !status.success() {
            return Err(format!("compilation command {cmd:?} failed, {status}").into());
        }
        Ok(())
    }

    fn to_command(&self) -> Command {
        let mut cmd = Command::new(&self.args[0]);
        cmd.args(&self.args[1..]);
        cmd
    }
}

fn get_var(name: &str, default: &str) -> Result<Vec<String>, Box<dyn Error>> {
    match env::var(name) {
        Ok(value) => Ok(shell_words::split(&value)?),
        Err(env::VarError::NotPresent) => Ok(shell_words::split(default)?),
        Err(err) => Err(format!("{name} {err}").into()),
    }
}

fn pkg_config_cflags(packages: &[&str]) -> Result<Vec<String>, Box<dyn Error>> {
    if packages.is_empty() {
        return Ok(Vec::new());
    }
    let pkg_config = env::var_os("PKG_CONFIG").unwrap_or_else(|| OsString::from("pkg-config"));
    let mut cmd = Command::new(pkg_config);
    cmd.arg("--cflags");
    cmd.args(packages);
    cmd.stderr(Stdio::inherit());
    let out = cmd.output()?;
    if !out.status.success() {
        let (status, stdout) = (out.status, String::from_utf8_lossy(&out.stdout));
        return Err(format!("command {cmd:?} failed, {status:?}\nstdout: {stdout}").into());
    }
    let stdout = str::from_utf8(&out.stdout)?;
    Ok(shell_words::split(stdout.trim())?)
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
struct Layout {
    size: usize,
    alignment: usize,
}

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
struct Results {
    /// Number of successfully completed tests.
    passed: usize,
    /// Total number of failed tests (including those that failed to compile).
    failed: usize,
}

impl Results {
    fn record_passed(&mut self) {
        self.passed += 1;
    }

    fn record_failed(&mut self) {
        self.failed += 1;
    }

    fn summary(&self) -> String {
        format!("{} passed; {} failed", self.passed, self.failed)
    }

    fn expect_total_success(&self) {
        if self.failed == 0 {
            println!("OK: {}", self.summary());
        } else {
            panic!("FAILED: {}", self.summary());
        };
    }
}

#[test]
fn cross_validate_constants_with_c() {
    let mut c_constants: Vec<(String, String)> = Vec::new();

    for l in get_c_output("constant").unwrap().lines() {
        let (name, value) = l.split_once(';').expect("Missing ';' separator");
        c_constants.push((name.to_owned(), value.to_owned()));
    }

    let mut results = Results::default();

    for ((rust_name, rust_value), (c_name, c_value)) in
        RUST_CONSTANTS.iter().zip(c_constants.iter())
    {
        if rust_name != c_name {
            results.record_failed();
            eprintln!("Name mismatch:\nRust: {rust_name:?}\nC:    {c_name:?}");
            continue;
        }

        if rust_value != c_value {
            results.record_failed();
            eprintln!(
                "Constant value mismatch for {rust_name}\nRust: {rust_value:?}\nC:    {c_value:?}",
            );
            continue;
        }

        results.record_passed();
    }

    results.expect_total_success();
}

#[test]
fn cross_validate_layout_with_c() {
    let mut c_layouts = Vec::new();

    for l in get_c_output("layout").unwrap().lines() {
        let (name, value) = l.split_once(';').expect("Missing first ';' separator");
        let (size, alignment) = value.split_once(';').expect("Missing second ';' separator");
        let size = size.parse().expect("Failed to parse size");
        let alignment = alignment.parse().expect("Failed to parse alignment");
        c_layouts.push((name.to_owned(), Layout { size, alignment }));
    }

    let mut results = Results::default();

    for ((rust_name, rust_layout), (c_name, c_layout)) in RUST_LAYOUTS.iter().zip(c_layouts.iter())
    {
        if rust_name != c_name {
            results.record_failed();
            eprintln!("Name mismatch:\nRust: {rust_name:?}\nC:    {c_name:?}");
            continue;
        }

        if rust_layout != c_layout {
            results.record_failed();
            eprintln!("Layout mismatch for {rust_name}\nRust: {rust_layout:?}\nC:    {c_layout:?}",);
            continue;
        }

        results.record_passed();
    }

    results.expect_total_success();
}

fn get_c_output(name: &str) -> Result<String, Box<dyn Error>> {
    let tmpdir = Builder::new().prefix("abi").tempdir()?;
    let exe = tmpdir.path().join(name);
    let c_file = Path::new("tests").join(name).with_extension("c");

    let cc = Compiler::new().expect("configured compiler");
    cc.compile(&c_file, &exe)?;

    let mut cmd = Command::new(exe);
    cmd.stderr(Stdio::inherit());
    let out = cmd.output()?;
    if !out.status.success() {
        let (status, stdout) = (out.status, String::from_utf8_lossy(&out.stdout));
        return Err(format!("command {cmd:?} failed, {status:?}\nstdout: {stdout}").into());
    }

    Ok(String::from_utf8(out.stdout)?)
}

const RUST_LAYOUTS: &[(&str, Layout)] = &[
    ("JSCCheckSyntaxMode", Layout {
        size: size_of::<JSCCheckSyntaxMode>(),
        alignment: align_of::<JSCCheckSyntaxMode>(),
    }),
    ("JSCCheckSyntaxResult", Layout {
        size: size_of::<JSCCheckSyntaxResult>(),
        alignment: align_of::<JSCCheckSyntaxResult>(),
    }),
    ("JSCClassClass", Layout {
        size: size_of::<JSCClassClass>(),
        alignment: align_of::<JSCClassClass>(),
    }),
    ("JSCClassVTable", Layout {
        size: size_of::<JSCClassVTable>(),
        alignment: align_of::<JSCClassVTable>(),
    }),
    ("JSCContextClass", Layout {
        size: size_of::<JSCContextClass>(),
        alignment: align_of::<JSCContextClass>(),
    }),
    ("JSCExceptionClass", Layout {
        size: size_of::<JSCExceptionClass>(),
        alignment: align_of::<JSCExceptionClass>(),
    }),
    ("JSCOptionType", Layout {
        size: size_of::<JSCOptionType>(),
        alignment: align_of::<JSCOptionType>(),
    }),
    ("JSCTypedArrayType", Layout {
        size: size_of::<JSCTypedArrayType>(),
        alignment: align_of::<JSCTypedArrayType>(),
    }),
    ("JSCValueClass", Layout {
        size: size_of::<JSCValueClass>(),
        alignment: align_of::<JSCValueClass>(),
    }),
    ("JSCValuePropertyFlags", Layout {
        size: size_of::<JSCValuePropertyFlags>(),
        alignment: align_of::<JSCValuePropertyFlags>(),
    }),
    ("JSCVirtualMachineClass", Layout {
        size: size_of::<JSCVirtualMachineClass>(),
        alignment: align_of::<JSCVirtualMachineClass>(),
    }),
    ("JSCWeakValueClass", Layout {
        size: size_of::<JSCWeakValueClass>(),
        alignment: align_of::<JSCWeakValueClass>(),
    }),
];

const RUST_CONSTANTS: &[(&str, &str)] = &[
    ("(gint) JSC_CHECK_SYNTAX_MODE_MODULE", "1"),
    ("(gint) JSC_CHECK_SYNTAX_MODE_SCRIPT", "0"),
    ("(gint) JSC_CHECK_SYNTAX_RESULT_IRRECOVERABLE_ERROR", "2"),
    ("(gint) JSC_CHECK_SYNTAX_RESULT_OUT_OF_MEMORY_ERROR", "4"),
    ("(gint) JSC_CHECK_SYNTAX_RESULT_RECOVERABLE_ERROR", "1"),
    ("(gint) JSC_CHECK_SYNTAX_RESULT_STACK_OVERFLOW_ERROR", "5"),
    ("(gint) JSC_CHECK_SYNTAX_RESULT_SUCCESS", "0"),
    ("(gint) JSC_CHECK_SYNTAX_RESULT_UNTERMINATED_LITERAL_ERROR", "3"),
    ("JSC_MAJOR_VERSION", "2"),
    ("JSC_MICRO_VERSION", "3"),
    ("JSC_MINOR_VERSION", "49"),
    ("JSC_OPTIONS_USE_DFG", "useDFGJIT"),
    ("JSC_OPTIONS_USE_FTL", "useFTLJIT"),
    ("JSC_OPTIONS_USE_JIT", "useJIT"),
    ("JSC_OPTIONS_USE_LLINT", "useLLInt"),
    ("(gint) JSC_OPTION_BOOLEAN", "0"),
    ("(gint) JSC_OPTION_DOUBLE", "4"),
    ("(gint) JSC_OPTION_INT", "1"),
    ("(gint) JSC_OPTION_RANGE_STRING", "6"),
    ("(gint) JSC_OPTION_SIZE", "3"),
    ("(gint) JSC_OPTION_STRING", "5"),
    ("(gint) JSC_OPTION_UINT", "2"),
    ("(gint) JSC_TYPED_ARRAY_FLOAT32", "10"),
    ("(gint) JSC_TYPED_ARRAY_FLOAT64", "11"),
    ("(gint) JSC_TYPED_ARRAY_INT16", "2"),
    ("(gint) JSC_TYPED_ARRAY_INT32", "3"),
    ("(gint) JSC_TYPED_ARRAY_INT64", "4"),
    ("(gint) JSC_TYPED_ARRAY_INT8", "1"),
    ("(gint) JSC_TYPED_ARRAY_NONE", "0"),
    ("(gint) JSC_TYPED_ARRAY_UINT16", "7"),
    ("(gint) JSC_TYPED_ARRAY_UINT32", "8"),
    ("(gint) JSC_TYPED_ARRAY_UINT64", "9"),
    ("(gint) JSC_TYPED_ARRAY_UINT8", "5"),
    ("(gint) JSC_TYPED_ARRAY_UINT8_CLAMPED", "6"),
    ("(guint) JSC_VALUE_PROPERTY_CONFIGURABLE", "1"),
    ("(guint) JSC_VALUE_PROPERTY_ENUMERABLE", "2"),
    ("(guint) JSC_VALUE_PROPERTY_WRITABLE", "4"),
];
