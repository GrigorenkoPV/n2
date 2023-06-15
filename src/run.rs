use crate::{load, progress::ConsoleProgress, terminal, trace, work};
use anyhow::anyhow;
use std::path::Path;

fn build(
    options: work::Options,
    build_filename: String,
    targets: Vec<String>,
    progress: &mut ConsoleProgress,
) -> anyhow::Result<Option<usize>> {
    let mut state = trace::scope("load::read", || load::read(&build_filename))?;
    let mut work = work::Work::new(
        state.graph,
        state.hashes,
        state.db,
        &options,
        progress,
        state.pools,
    );

    // Attempt to rebuild build.ninja.
    if let Some(target) = work.is_build_target(&build_filename) {
        work.want_fileid(target)?;
        match trace::scope("work.run", || work.run())? {
            None => return Ok(None),
            Some(0) => {
                // build.ninja already up to date.
                // TODO: this logic is not right in the case where a build has
                // a step that doesn't touch build.ninja.
            }
            Some(_) => {
                // Regenerated build.ninja; start over.
                state = trace::scope("load::read", || load::read(&build_filename))?;
                work = work::Work::new(
                    state.graph,
                    state.hashes,
                    state.db,
                    &options,
                    progress,
                    state.pools,
                );
            }
        }
    }

    if !targets.is_empty() {
        for name in &targets {
            work.want_file(name)?;
        }
    } else if !state.default.is_empty() {
        for target in state.default {
            work.want_fileid(target)?;
        }
    } else {
        anyhow::bail!("no path specified and no default");
    }

    trace::scope("work.run", || work.run())
}

fn default_parallelism() -> anyhow::Result<usize> {
    // Ninja uses available processors + a constant, but I don't think the
    // difference matters too much.
    let par = std::thread::available_parallelism()?;
    Ok(usize::from(par))
}

#[derive(argh::FromArgs)] // this struct generates the flags and --help output
/// n2, a ninja compatible build system
struct Args {
    /// chdir before running
    #[argh(option, short = 'C')]
    chdir: Option<String>,

    /// input build file [default=build.ninja]
    #[argh(option, short = 'f', default = "(\"build.ninja\".into())")]
    build_file: String,

    /// debugging tools
    #[argh(option, short = 'd')]
    debug: Option<String>,

    /// subcommands
    #[argh(option, short = 't')]
    tool: Option<String>,

    /// parallelism [default uses system thread count]
    #[argh(option, short = 'j')] // tododefault_parallelism()")]
    parallelism: Option<usize>,

    /// keep going until at least N failures (0 means infinity) [default=1]
    #[argh(option, short = 'k', default = "1")]
    keep_going: usize,

    /// print version (required by cmake)
    #[argh(switch, hidden_help)]
    version: bool,

    /// print executed command lines
    #[argh(switch, short = 'v')]
    verbose: bool,

    /// targets to build
    #[argh(positional)]
    targets: Vec<String>,
}

fn run_impl() -> anyhow::Result<i32> {
    let fake_ninja_compat = Path::new(&std::env::args().nth(0).unwrap())
        .file_name()
        .unwrap()
        == std::ffi::OsStr::new(&format!("ninja{}", std::env::consts::EXE_SUFFIX));

    let args: Args = argh::from_env();

    let mut options = work::Options {
        parallelism: match args.parallelism {
            Some(p) => p,
            None => default_parallelism()?,
        },
        keep_going: args.keep_going,
        explain: false,
    };

    if args.version {
        if fake_ninja_compat {
            println!("1.10.2");
        } else {
            println!("{}", env!("CARGO_PKG_VERSION"));
        }
        return Ok(0);
    }

    if let Some(debug) = args.debug {
        match debug.as_str() {
            "explain" => options.explain = true,
            "list" => {
                println!("debug tools:");
                println!("  explain  print why each target is considered out of date");
                println!("  trace    generate json performance trace");
                return Ok(1);
            }
            "trace" => trace::open("trace.json")?,
            _ => anyhow::bail!("unknown -d {:?}, use -d list to list", debug),
        }
    }

    if let Some(tool) = args.tool {
        match tool.as_str() {
            "list" => {
                println!("subcommands:");
                println!("  (none yet, but see README if you're looking here trying to get CMake to work)");
                return Ok(1);
            }
            _ => {
                if fake_ninja_compat {
                    return Ok(0);
                }
                anyhow::bail!("unknown -t {:?}, use -t list to list", tool);
            }
        }
    }

    if let Some(dir) = args.chdir {
        let dir = Path::new(&dir);
        std::env::set_current_dir(dir).map_err(|err| anyhow!("chdir {:?}: {}", dir, err))?;
    }

    let mut progress: ConsoleProgress = ConsoleProgress::new(args.verbose, terminal::use_fancy());
    match build(options, args.build_file, args.targets, &mut progress)? {
        None => {
            // Don't print any summary, the failing task is enough info.
            return Ok(1);
        }
        Some(0) => {
            // Special case: don't print numbers when no work done.
            println!("n2: no work to do");
        }
        Some(n) => {
            println!(
                "n2: ran {} task{}, now up to date",
                n,
                if n == 1 { "" } else { "s" }
            );
        }
    }

    Ok(0)
}

pub fn run() -> anyhow::Result<i32> {
    let res = run_impl();
    trace::close();
    res
}
