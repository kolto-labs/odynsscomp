//! Command line front end for [`odynsscomp`].
//!
//! The argument grammar of `nwnnsscomp.exe` is accepted as-is, so existing
//! scripts and tools keep working:
//!
//! ```text
//! nwnnsscomp [-cd] infile [-o outfile]
//! ```
//!
//! Longer options are added on top for the things the original guessed at,
//! chiefly which game to compile for and where `nwscript.nss` lives.

use std::path::PathBuf;
use std::process::ExitCode;

use odynsscomp::{compile_file, CompileOptions, Game, NwScript, OptimizationLevel};

const USAGE: &str = "\
odynsscomp 1.0.0 - NWScript compiler for Knights of the Old Republic I and II

Usage:
  odynsscomp [-cd] infile [-o outfile]

  infile   - name of the input file.
  outfile  - name of the output file.

  -c            Compile the script. This is the default.
  -d            Decompile the script. Not supported.
  -o <file>     Write the compiled script here.
  -g, --game <1|2>       Which game to compile for. Defaults to 1.
  -n, --nwscript <file>  Path to that game's nwscript.nss.
  -i, --include <dir>    Directory to search for included scripts.
                         Repeat the flag to add more.
  -O, --optimize         Fold constants and drop unreachable code.
  -h, --help             Show this help.
  -v, --version          Show the version.

odynsscomp needs the nwscript.nss from the game you are compiling for.
Engine functions are called by index, and the indexes differ between the two
games. Pass --nwscript, or put nwscript.nss next to odynsscomp and it will be
picked up the way the original compiler did.
";

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();

    match Args::parse(argv) {
        Ok(Args::Help) => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        Ok(Args::Version) => {
            println!("odynsscomp {}", odynsscomp::VERSION);
            ExitCode::SUCCESS
        }
        Ok(Args::Compile(job)) => match run(job) {
            Ok(()) => ExitCode::SUCCESS,
            Err(message) => {
                eprintln!("{message}");
                ExitCode::FAILURE
            }
        },
        Err(message) => {
            eprintln!("{message}");
            eprintln!();
            eprint!("{USAGE}");
            ExitCode::FAILURE
        }
    }
}

fn run(job: Job) -> Result<(), String> {
    let nwscript_path = match job.nwscript {
        Some(path) => path,
        None => default_nwscript_path()
            .ok_or_else(|| "Error: unable to locate nwscript.nss. Pass --nwscript.".to_string())?,
    };

    let nwscript = NwScript::load(job.game, &nwscript_path).map_err(|error| error.to_string())?;

    let mut options = CompileOptions::new(&nwscript);
    for directory in &job.include_paths {
        options = options.with_include_path(directory.clone());
    }
    if job.optimize {
        options = options.with_optimization(OptimizationLevel::Basic);
    }

    let bytes = compile_file(&job.input, &options).map_err(|error| error.to_string())?;

    let output = job
        .output
        .unwrap_or_else(|| job.input.with_extension("ncs"));

    std::fs::write(&output, &bytes)
        .map_err(|error| format!("Error: unable to write \"{}\": {error}", output.display()))?;

    println!("Compiled {} to {}", job.input.display(), output.display());
    Ok(())
}

/// Looks for `nwscript.nss` beside the running binary.
///
/// The original compiler required it there, and a lot of modding setups still
/// lay their tools out that way.
fn default_nwscript_path() -> Option<PathBuf> {
    let executable = std::env::current_exe().ok()?;
    let candidate = executable.parent()?.join("nwscript.nss");
    if candidate.is_file() {
        Some(candidate)
    } else {
        None
    }
}

/// What the command line asked for.
#[derive(Debug, PartialEq)]
enum Args {
    Help,
    Version,
    Compile(Job),
}

/// One compile.
#[derive(Debug, Default, PartialEq)]
struct Job {
    input: PathBuf,
    output: Option<PathBuf>,
    game: Game,
    nwscript: Option<PathBuf>,
    include_paths: Vec<PathBuf>,
    optimize: bool,
}

impl Args {
    /// Parses arguments, with `argv[0]` already removed.
    fn parse(argv: Vec<String>) -> Result<Args, String> {
        if argv.is_empty() {
            return Ok(Args::Help);
        }

        let mut job = Job::default();
        let mut input: Option<PathBuf> = None;
        let mut index = 0usize;

        while index < argv.len() {
            let arg = argv[index].clone();
            index += 1;

            let mut next = |flag: &str| -> Result<String, String> {
                let value = argv
                    .get(index)
                    .cloned()
                    .ok_or_else(|| format!("Error: {flag} needs a value"))?;
                index += 1;
                Ok(value)
            };

            match arg.as_str() {
                "-h" | "--help" | "/?" => return Ok(Args::Help),
                "-v" | "--version" => return Ok(Args::Version),
                "-c" | "--compile" => {}
                "-d" | "--decompile" => {
                    return Err("Error: decompiling is not supported".to_string());
                }
                "-o" | "--output" => job.output = Some(PathBuf::from(next("-o")?)),
                "-g" | "--game" => {
                    let value = next("--game")?;
                    job.game = Game::parse(&value)
                        .ok_or_else(|| format!("Error: unknown game \"{value}\""))?;
                }
                "-n" | "--nwscript" => job.nwscript = Some(PathBuf::from(next("--nwscript")?)),
                "-i" | "--include" => job.include_paths.push(PathBuf::from(next("--include")?)),
                "-O" | "--optimize" => job.optimize = true,
                other if other.starts_with('-') && other.len() > 1 => {
                    return Err(format!("Error: unrecognized option \"{other}\""));
                }
                other => {
                    if input.is_none() {
                        input = Some(PathBuf::from(other));
                    } else if job.output.is_none() {
                        // The original accepts a bare second path as the output.
                        job.output = Some(PathBuf::from(other));
                    } else {
                        return Err("Error: too many arguments".to_string());
                    }
                }
            }
        }

        job.input = input.ok_or_else(|| "Error: no input file".to_string())?;
        Ok(Args::Compile(job))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Result<Args, String> {
        Args::parse(list.iter().map(|item| item.to_string()).collect())
    }

    fn job(list: &[&str]) -> Job {
        match args(list).expect("arguments parse") {
            Args::Compile(job) => job,
            other => panic!("expected a compile job, got {other:?}"),
        }
    }

    #[test]
    fn no_arguments_shows_help() {
        assert_eq!(args(&[]), Ok(Args::Help));
    }

    #[test]
    fn the_original_argument_order_still_works() {
        let job = job(&["-c", "k_test.nss", "-o", "k_test.ncs"]);
        assert_eq!(job.input, PathBuf::from("k_test.nss"));
        assert_eq!(job.output, Some(PathBuf::from("k_test.ncs")));
        assert_eq!(job.game, Game::K1);
    }

    #[test]
    fn a_bare_second_path_is_the_output() {
        let job = job(&["k_test.nss", "k_test.ncs"]);
        assert_eq!(job.output, Some(PathBuf::from("k_test.ncs")));
    }

    #[test]
    fn the_game_can_be_chosen() {
        assert_eq!(job(&["--game", "2", "k_test.nss"]).game, Game::K2);
        assert_eq!(job(&["-g", "tsl", "k_test.nss"]).game, Game::K2);
    }

    #[test]
    fn include_directories_stack_up_in_order() {
        let job = job(&["-i", "first", "-i", "second", "k_test.nss"]);
        assert_eq!(job.include_paths.len(), 2);
        assert!(job.include_paths[0].ends_with("first"));
    }

    #[test]
    fn decompiling_is_refused_clearly() {
        let error = args(&["-d", "k_test.ncs"]).expect_err("decompiling is refused");
        assert!(error.contains("not supported"));
    }

    #[test]
    fn an_unknown_option_is_refused() {
        assert!(args(&["-z", "k_test.nss"]).is_err());
    }

    #[test]
    fn a_missing_input_is_refused() {
        assert!(args(&["-c"]).is_err());
    }

    #[test]
    fn an_unknown_game_is_refused() {
        assert!(args(&["-g", "k3", "k_test.nss"]).is_err());
    }

    #[test]
    fn help_and_version_win_over_everything_else() {
        assert_eq!(args(&["-c", "x.nss", "--help"]), Ok(Args::Help));
        assert_eq!(args(&["--version"]), Ok(Args::Version));
    }
}
