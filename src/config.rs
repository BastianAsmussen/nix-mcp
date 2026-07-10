//! Static registry of Nix configuration settings.
//!
//! The key list and descriptions are captured once from `nix config show --json`
//! (Nix 2.34.7). At runtime the worker reads live values via
//! [`nix_bindings::Context::get_setting`]; this module only supplies the
//! metadata that the C API cannot enumerate.

/// Metadata for one `nix.conf` setting, extracted at build time.
pub struct SettingMeta {
    /// The canonical setting key (e.g. `"connect-timeout"`).
    pub key: &'static str,
    /// Deprecated or alternative names for this setting.
    pub aliases: &'static [&'static str],
    /// One-paragraph description (markdown stripped, whitespace-normalized).
    pub description: &'static str,
    /// The experimental feature gate, if any.
    pub experimental_feature: Option<&'static str>,
}

// Generated from `nix config show --json` (Nix 2.34.7, 129 settings).
// To regenerate: `nix config show --json` and run the extraction script
// documented in the commit that introduced this file.
pub static KNOWN_SETTINGS: &[SettingMeta] = &[
    SettingMeta {
        key: "abort-on-warn",
        aliases: &[],
        description: "If set to true, builtins.warn throws an error when logging a warning. This will give you a stack trace that leads to the location of the warning. This is useful for finding information about warnings in third-party Nix code when you can not start the interactive debugger, such as when Nix is call...",
        experimental_feature: None,
    },
    SettingMeta {
        key: "accept-flake-config",
        aliases: &[],
        description: "Whether to accept Nix configuration settings from a flake without prompting.",
        experimental_feature: Some("flakes"),
    },
    SettingMeta {
        key: "access-tokens",
        aliases: &[],
        description: "Access tokens used to access protected GitHub, GitLab, or other locations requiring token-based authentication. Access tokens are specified as a string made up of space-separated host=token values. The specific token used is selected by matching the host portion against the \"host\" specification o...",
        experimental_feature: None,
    },
    SettingMeta {
        key: "allow-dirty",
        aliases: &[],
        description: "Whether to allow dirty Git/Mercurial trees.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "allow-dirty-locks",
        aliases: &[],
        description: "Whether to allow dirty inputs (such as dirty Git workdirs) to be locked via their NAR hash. This is generally bad practice since Nix has no way to obtain such inputs if they are subsequently modified. Therefore lock files with dirty locks should generally only be used for local testing, and shoul...",
        experimental_feature: Some("flakes"),
    },
    SettingMeta {
        key: "allow-import-from-derivation",
        aliases: &[],
        description: "By default, Nix allows Import from Derivation. With this option set to false, Nix throws an error when evaluating an expression that uses this feature, even when the required store object is readily available. This ensures that evaluation doesn't require any builds to take place, regardless of th...",
        experimental_feature: None,
    },
    SettingMeta {
        key: "allow-new-privileges",
        aliases: &[],
        description: "(Linux-specific.) By default, builders on Linux cannot acquire new privileges by calling setuid/setgid programs or programs that have file capabilities. For example, programs such as sudo or ping should fail. (Note that in sandbox builds, no such programs are available unless you bind-mount them ...",
        experimental_feature: None,
    },
    SettingMeta {
        key: "allow-symlinked-store",
        aliases: &[],
        description: "If set to true, Nix stops complaining if the store directory (typically /nix/store) contains symlink components. This risks making some builds \"impure\" because builders sometimes \"canonicalise\" paths by resolving all symlink components. Problems occur if those builds are then deployed to machines...",
        experimental_feature: None,
    },
    SettingMeta {
        key: "allow-unsafe-native-code-during-evaluation",
        aliases: &[],
        description: "Enable built-in functions that allow executing native code. In particular, this adds: - builtins.importNative *path* *symbol* Opens dynamic shared object (DSO) at *path*, loads the function with the symbol name *symbol* from it and runs it. The loaded function must have the following signature: T...",
        experimental_feature: None,
    },
    SettingMeta {
        key: "allowed-impure-host-deps",
        aliases: &[],
        description: "Which prefixes to allow derivations to ask for access to (primarily for Darwin).",
        experimental_feature: None,
    },
    SettingMeta {
        key: "allowed-uris",
        aliases: &[],
        description: "A list of URI prefixes to which access is allowed in restricted evaluation mode. For example, when set to https://github.com/NixOS, builtin functions such as fetchGit are allowed to access https://github.com/NixOS/patchelf.git. Access is granted when - the URI is equal to the prefix, - or the URI...",
        experimental_feature: None,
    },
    SettingMeta {
        key: "allowed-users",
        aliases: &[],
        description: "A list of user names, separated by whitespace. These users are allowed to connect to the Nix daemon. You can specify groups by prefixing names with @. For instance, @wheel means all users in the wheel group. Also, you can allow all users by specifying *.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "always-allow-substitutes",
        aliases: &[],
        description: "If set to true, Nix ignores the allowSubstitutes attribute in derivations and always attempt to use available substituters.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "auto-allocate-uids",
        aliases: &[],
        description: "Whether to select UIDs for builds automatically, instead of using the users in build-users-group. UIDs are allocated starting at 872415232 (0x34000000) on Linux and 56930 on macOS.",
        experimental_feature: Some("auto-allocate-uids"),
    },
    SettingMeta {
        key: "auto-optimise-store",
        aliases: &[],
        description: "If set to true, Nix automatically detects files in the store that have identical contents, and replaces them with hard links to a single copy. This saves disk space. If set to false (the default), you can still run nix-store --optimise to get rid of duplicate files.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "bash-prompt",
        aliases: &[],
        description: "The bash prompt (PS1) in nix develop shells.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "bash-prompt-prefix",
        aliases: &[],
        description: "Prefix prepended to the PS1 environment variable in nix develop shells.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "bash-prompt-suffix",
        aliases: &[],
        description: "Suffix appended to the PS1 environment variable in nix develop shells.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "build-dir",
        aliases: &[],
        description: "Override the build-dir store setting for all stores that have this setting. See also the per-store build-dir setting.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "build-hook",
        aliases: &[],
        description: "The path to the helper program that executes remote builds. Nix communicates with the build hook over stdio using a custom protocol to request builds that cannot be performed directly by the Nix daemon. The default value is the internal Nix binary that implements remote building.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "build-poll-interval",
        aliases: &[],
        description: "How often (in seconds) to poll for locks.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "build-users-group",
        aliases: &[],
        description: "This options specifies the Unix group containing the Nix build user accounts. In multi-user Nix installations, builds should not be performed by the Nix account since that would allow users to arbitrarily modify the Nix store and database by supplying specially crafted builders; and they cannot b...",
        experimental_feature: None,
    },
    SettingMeta {
        key: "builders",
        aliases: &[],
        description: "A semicolon- or newline-separated list of build machines.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "builders-use-substitutes",
        aliases: &[],
        description: "If set to true, Nix instructs remote build machines to use their own substituters if available. It means that remote build hosts fetch as many dependencies as possible from their own substituters (e.g, from cache.nixos.org) instead of waiting for the local machine to upload them all. This can dra...",
        experimental_feature: None,
    },
    SettingMeta {
        key: "commit-lock-file-summary",
        aliases: &["commit-lockfile-summary"],
        description: "The commit summary to use when committing changed flake lock files. If empty, the summary is generated based on the action performed.",
        experimental_feature: Some("flakes"),
    },
    SettingMeta {
        key: "compress-build-log",
        aliases: &["build-compress-log"],
        description: "If set to true (the default), build logs written to /nix/var/log/nix/drvs are compressed on the fly using bzip2. Otherwise, they are not compressed.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "connect-timeout",
        aliases: &[],
        description: "The timeout (in seconds) for establishing connections in the binary cache substituter. It corresponds to curl's --connect-timeout option. A value of 0 means no limit.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "cores",
        aliases: &["build-cores"],
        description: "Sets the value of the NIX_BUILD_CORES environment variable in the invocation of the builder executable of a derivation. The builder executable can use this variable to control its own maximum amount of parallelism.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "debugger-on-trace",
        aliases: &[],
        description: "If set to true and the --debugger flag is given, the following functions enter the debugger like builtins.break: builtins.trace, builtins.traceVerbose if trace-verbose is set to true, and builtins.warn. This is useful for debugging warnings in third-party Nix code.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "debugger-on-warn",
        aliases: &[],
        description: "If set to true and the --debugger flag is given, builtins.warn will enter the debugger like builtins.break. This is useful for debugging warnings in third-party Nix code. Use debugger-on-trace to also enter the debugger on legacy warnings that are logged with builtins.trace.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "diff-hook",
        aliases: &[],
        description: "Absolute path to an executable capable of diffing build results. The hook is executed if run-diff-hook is true, and the output of a build is known to not be the same.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "download-attempts",
        aliases: &[],
        description: "The number of times Nix attempts to download a file before giving up.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "download-buffer-size",
        aliases: &[],
        description: "The size of Nix's internal download buffer in bytes during curl transfers. If data is not processed quickly enough to exceed the size of this buffer, downloads may stall. The default is 1048576 (1 MiB).",
        experimental_feature: None,
    },
    SettingMeta {
        key: "download-speed",
        aliases: &[],
        description: "Specify the maximum transfer rate in kilobytes per second you want Nix to use for downloads.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "eval-attrset-update-layer-rhs-threshold",
        aliases: &[],
        description: "Tunes the maximum size of an attribute set that, when used as a right operand in an attribute set update expression, uses a more space-efficient linked-list representation of attribute sets.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "eval-cache",
        aliases: &[],
        description: "Whether to use the flake evaluation cache. Certain commands won't have to evaluate when invoked for the second time with a particular version of a flake. Intermediate results are not cached.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "eval-profile-file",
        aliases: &[],
        description: "Specifies the file where evaluation profile is saved.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "eval-profiler",
        aliases: &[],
        description: "Enables evaluation profiling. The following modes are supported: flamegraph stack sampling profiler. Outputs folded format, one line per stack (suitable for flamegraph.pl and compatible tools). Use eval-profile-file to specify where the profile is saved.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "eval-profiler-frequency",
        aliases: &[],
        description: "Specifies the sampling rate in hertz for sampling evaluation profilers. Use 0 to sample the stack after each function call. See eval-profiler.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "eval-system",
        aliases: &[],
        description: "This option defines builtins.currentSystem in the Nix language if it is set as a non-empty string. Otherwise, if it is defined as the empty string (the default), the value of the system configuration setting is used instead. Unlike system, this setting does not change what kind of derivations ca...",
        experimental_feature: None,
    },
    SettingMeta {
        key: "experimental-features",
        aliases: &[],
        description: "Experimental features that are enabled.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "external-builders",
        aliases: &[],
        description: "Helper programs that execute derivations.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "extra-platforms",
        aliases: &[],
        description: "System types of executables that can be run on this machine. Nix only builds a given store derivation locally when its system attribute equals any of the values specified here or in the system option.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "fallback",
        aliases: &["build-fallback"],
        description: "If set to true, Nix falls back to building from source if a binary substitute fails. This is equivalent to the --fallback flag. The default is false.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "filter-syscalls",
        aliases: &[],
        description: "Whether to prevent certain dangerous system calls, such as creation of setuid/setgid files or adding ACLs or extended attributes. Only disable this if you're aware of the security implications.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "flake-registry",
        aliases: &[],
        description: "Path or URI of the global flake registry. When empty, disables the global flake registry.",
        experimental_feature: Some("flakes"),
    },
    SettingMeta {
        key: "fsync-metadata",
        aliases: &[],
        description: "If set to true, changes to the Nix store metadata (in /nix/var/nix/db) are synchronously flushed to disk. This improves robustness in case of system crashes, but reduces performance. The default is true.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "fsync-store-paths",
        aliases: &[],
        description: "Whether to call fsync() on store paths before registering them, to flush them to disk. This improves robustness in case of system crashes, but reduces performance. The default is false.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "gc-reserved-space",
        aliases: &[],
        description: "Amount of reserved disk space for the garbage collector.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "hashed-mirrors",
        aliases: &[],
        description: "A list of web servers used by builtins.fetchurl to obtain files by hash.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "http-connections",
        aliases: &["binary-caches-parallel-connections"],
        description: "The maximum number of parallel TCP connections used to fetch files from binary caches and by other downloads. It defaults to 25. 0 means no limit.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "http2",
        aliases: &[],
        description: "Whether to enable HTTP/2 support.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "id-count",
        aliases: &[],
        description: "The number of UIDs/GIDs to use for dynamic ID allocation.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "ignore-try",
        aliases: &[],
        description: "If set to true, ignore exceptions inside 'tryEval' calls when evaluating Nix expressions in debug mode (using the --debugger flag). By default the debugger pauses on all exceptions.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "ignored-acls",
        aliases: &[],
        description: "A list of ACLs that should be ignored, normally Nix attempts to remove all ACLs from files and directories in the Nix store, but some ACLs like security.selinux or system.nfs4_acl can't be removed even by root. Therefore it's best to just ignore them.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "impersonate-linux-26",
        aliases: &["build-impersonate-linux-26"],
        description: "Whether to impersonate a Linux 2.6 machine on newer kernels.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "impure-env",
        aliases: &[],
        description: "A list of items, each in the format of name=value: Set environment variable name to value.",
        experimental_feature: Some("configurable-impure-env"),
    },
    SettingMeta {
        key: "json-log-path",
        aliases: &[],
        description: "A file or unix socket to which JSON records of Nix's log output are written, in the same format as --log-format internal-json (without the @nix prefixes on each line).",
        experimental_feature: None,
    },
    SettingMeta {
        key: "keep-build-log",
        aliases: &["build-keep-log"],
        description: "If set to true (the default), Nix writes the build log of a derivation (i.e. the standard output and error of its builder) to the directory /nix/var/log/nix/drvs. The build log can be retrieved using the command nix-store -l path.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "keep-derivations",
        aliases: &["gc-keep-derivations"],
        description: "If true (default), the garbage collector keeps the derivations from which non-garbage store paths were built. If false, they are deleted unless explicitly registered as a root (or reachable from other roots).",
        experimental_feature: None,
    },
    SettingMeta {
        key: "keep-env-derivations",
        aliases: &["env-keep-derivations"],
        description: "If false (default), derivations are not stored in Nix user environments. That is, the derivations of any build-time-only dependencies may be garbage-collected. If true, when you add a Nix derivation to a user environment, the path of the derivation is stored in the user environment.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "keep-failed",
        aliases: &[],
        description: "Whether to keep temporary directories of failed builds.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "keep-going",
        aliases: &[],
        description: "Whether to keep building derivations when another build fails.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "keep-outputs",
        aliases: &["gc-keep-outputs"],
        description: "If true, the garbage collector keeps the outputs of non-garbage derivations. If false (default), outputs are deleted unless they are GC roots themselves (or reachable from other roots).",
        experimental_feature: None,
    },
    SettingMeta {
        key: "lint-absolute-path-literals",
        aliases: &[],
        description: "Controls handling of absolute path literals (paths starting with /) and home path literals (paths starting with ~/). Values: ignore (default), warn, fatal.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "lint-short-path-literals",
        aliases: &[],
        description: "Controls handling of relative path literals that don't start with ./ or ../. Values: ignore (default), warn, fatal. For example, with this setting set to warn or fatal, foo/bar would suggest using ./foo/bar instead.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "lint-url-literals",
        aliases: &[],
        description: "Controls handling of unquoted URLs as part of the Nix language syntax. Values: ignore, warn, fatal. Unquoted URLs are being deprecated and their usage is discouraged.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "log-lines",
        aliases: &[],
        description: "The number of lines of the tail of the log to show if a build fails.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "max-build-log-size",
        aliases: &["build-max-log-size"],
        description: "This option defines the maximum number of bytes that a builder can write to its stdout/stderr. If the builder exceeds this limit, it's killed. A value of 0 (the default) means that there is no limit.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "max-call-depth",
        aliases: &[],
        description: "The maximum function call depth to allow before erroring.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "max-free",
        aliases: &[],
        description: "When a garbage collection is triggered by the min-free option, it stops as soon as max-free bytes are available. The default is infinity (i.e. delete all garbage).",
        experimental_feature: None,
    },
    SettingMeta {
        key: "max-jobs",
        aliases: &["build-max-jobs"],
        description: "Maximum number of jobs that Nix tries to build locally in parallel. The special value auto causes Nix to use the number of CPUs in your system. Use 0 to disable local builds and directly use the remote machines specified in builders.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "max-silent-time",
        aliases: &["build-max-silent-time"],
        description: "This option defines the maximum number of seconds that a builder can go without producing any data on standard output or standard error. This is useful (for instance in an automated build system) to catch builds that are stuck in an infinite loop, or to catch remote builds that are hanging due to...",
        experimental_feature: None,
    },
    SettingMeta {
        key: "max-substitution-jobs",
        aliases: &["substitution-max-jobs"],
        description: "This option defines the maximum number of substitution jobs that Nix tries to run in parallel. The default is 16. The minimum value one can choose is 1 and lower values are interpreted as 1.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "min-free",
        aliases: &[],
        description: "When free disk space in /nix/store drops below min-free during a build, Nix performs a garbage-collection until max-free bytes are available or there is no more garbage. A value of 0 (the default) disables this feature.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "min-free-check-interval",
        aliases: &[],
        description: "Number of seconds between checking free disk space.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "nar-buffer-size",
        aliases: &[],
        description: "Maximum size of NARs before spilling them to disk.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "narinfo-cache-meta-ttl",
        aliases: &[],
        description: "The TTL in seconds for caching binary cache metadata (i.e. /nix-cache-info).",
        experimental_feature: None,
    },
    SettingMeta {
        key: "narinfo-cache-negative-ttl",
        aliases: &[],
        description: "The TTL in seconds for negative lookups. If a store path is queried from a substituter but was not found, a negative lookup is cached in the local disk cache database for the specified duration.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "narinfo-cache-positive-ttl",
        aliases: &[],
        description: "The TTL in seconds for positive lookups. If a store path is queried from a substituter, the result of the query is cached in the local disk cache database including some of the NAR metadata.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "netrc-file",
        aliases: &[],
        description: "If set to an absolute path to a netrc file, Nix uses the HTTP authentication credentials in this file when trying to download from a remote host through HTTP or HTTPS. Defaults to $NIX_CONF_DIR/netrc.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "nix-path",
        aliases: &[],
        description: "List of search paths to use for lookup path resolution. This setting determines the value of builtins.nixPath and can be used with builtins.findFile.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "nix-shell-always-looks-for-shell-nix",
        aliases: &[],
        description: "Before Nix 2.24, nix-shell would only look at shell.nix if it was in the working directory. Since Nix 2.24, nix-shell always looks for a shell.nix, whether that's in the working directory, or in a directory that was passed as an argument.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "nix-shell-shebang-arguments-relative-to-script",
        aliases: &[],
        description: "Before Nix 2.24, relative file path expressions in arguments in a nix-shell shebang were resolved relative to the working directory. Since Nix 2.24, nix-shell resolves these paths in a manner that is relative to the script's directory.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "plugin-files",
        aliases: &[],
        description: "A list of plugin files to be loaded by Nix. Each of these files is dlopened by Nix.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "post-build-hook",
        aliases: &[],
        description: "Optional. The path to a program to execute after each build. This option is only settable in the global nix.conf, or on the command line by trusted users.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "pre-build-hook",
        aliases: &[],
        description: "If set, the path to a program that can set extra derivation-specific settings for this system. This is used for settings that can't be captured by the derivation model itself and are too variable between different versions of the same system to be hard-coded into nix.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "preallocate-contents",
        aliases: &[],
        description: "Whether to preallocate files when writing objects with known size.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "print-missing",
        aliases: &[],
        description: "Whether to print what paths need to be built or downloaded.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "pure-eval",
        aliases: &[],
        description: "Pure evaluation mode ensures that the result of Nix expressions is fully determined by explicitly declared inputs, and not influenced by external state.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "require-drop-supplementary-groups",
        aliases: &[],
        description: "Following the principle of least privilege, Nix attempts to drop supplementary groups when building with sandboxing. However this can fail under some circumstances.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "require-sigs",
        aliases: &[],
        description: "If set to true (the default), any non-content-addressed path added or copied to the Nix store (e.g. when substituting from a binary cache) must have a signature by a trusted key.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "restrict-eval",
        aliases: &[],
        description: "If set to true, the Nix evaluator doesn't allow access to any files outside of builtins.nixPath, or to URIs outside of allowed-uris.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "run-diff-hook",
        aliases: &[],
        description: "If true, enable the execution of the diff-hook program. When using the Nix daemon, run-diff-hook must be set in the nix.conf configuration file, and cannot be passed at the command line.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "sandbox",
        aliases: &["build-use-chroot", "build-use-sandbox"],
        description: "If set to true, builds are performed in a sandboxed environment, i.e., they're isolated from the normal file system hierarchy and only see their dependencies in the Nix store, the temporary build directory, private versions of /proc, /dev, /dev/shm and /dev/pts (on Linux), and the paths configur...",
        experimental_feature: None,
    },
    SettingMeta {
        key: "sandbox-build-dir",
        aliases: &[],
        description: "Linux only. The build directory inside the sandbox. This directory is backed by build-dir on the host.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "sandbox-dev-shm-size",
        aliases: &[],
        description: "Linux only. This option determines the maximum size of the tmpfs filesystem mounted on /dev/shm in Linux sandboxes. The default is 50%.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "sandbox-fallback",
        aliases: &[],
        description: "Whether to disable sandboxing when the kernel doesn't allow it.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "sandbox-paths",
        aliases: &["build-chroot-dirs", "build-sandbox-paths"],
        description: "A list of paths bind-mounted into Nix sandbox environments. You can use the syntax target=source to mount a path in a different location in the sandbox.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "secret-key-files",
        aliases: &[],
        description: "A whitespace-separated list of files containing secret (private) keys. These are used to sign locally-built paths. They can be generated using nix-store --generate-binary-cache-key.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "show-trace",
        aliases: &[],
        description: "Whether Nix should print out a stack trace in case of Nix expression evaluation errors.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "ssl-cert-file",
        aliases: &[],
        description: "The path of a file containing CA certificates used to authenticate https:// downloads.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "stalled-download-timeout",
        aliases: &[],
        description: "The timeout (in seconds) for receiving data from servers during download. Nix cancels idle downloads after this timeout's duration.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "start-id",
        aliases: &[],
        description: "The first UID and GID to use for dynamic ID allocation.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "store",
        aliases: &[],
        description: "The URL of the Nix store to use for most operations. See the Store Types section of the manual for supported store types and settings.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "substitute",
        aliases: &["build-use-substitutes"],
        description: "If set to true (default), Nix uses binary substitutes if available. This option can be disabled to force building from source.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "substituters",
        aliases: &["binary-caches"],
        description: "A list of URLs of Nix stores to be used as substituters, separated by whitespace. A substituter is an additional store from which Nix can obtain store objects instead of building them.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "sync-before-registering",
        aliases: &[],
        description: "Whether to call sync() before registering a path as valid.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "system",
        aliases: &[],
        description: "The system type of the current Nix installation. Nix only builds a given store derivation locally when its system attribute equals any of the values specified here or in extra-platforms.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "system-features",
        aliases: &[],
        description: "A set of system \"features\" supported by this machine. This complements the system and extra-platforms configuration options and the corresponding system attribute on derivations.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "tarball-ttl",
        aliases: &[],
        description: "The number of seconds a downloaded tarball is considered fresh. If the cached tarball is stale, Nix checks whether it is still up to date using the ETag header.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "timeout",
        aliases: &["build-timeout"],
        description: "This option defines the maximum number of seconds that a builder can run. This is useful (for instance in an automated build system) to catch builds that are stuck in an infinite loop but keep writing to their standard output or standard error.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "trace-function-calls",
        aliases: &[],
        description: "If set to true, the Nix evaluator traces every function call. Nix prints a log message at the \"vomit\" level for every function entrance and function exit.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "trace-import-from-derivation",
        aliases: &[],
        description: "By default, Nix allows Import from Derivation. When this setting is true, Nix logs a warning indicating that it performed such an import. This option has no effect if allow-import-from-derivation is disabled.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "trace-verbose",
        aliases: &[],
        description: "Whether builtins.traceVerbose should trace its first argument when evaluated.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "trust-tarballs-from-git-forges",
        aliases: &[],
        description: "If enabled (the default), Nix considers tarballs from GitHub and similar Git forges to be locked if a Git revision is specified.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "trusted-public-keys",
        aliases: &["binary-cache-public-keys"],
        description: "A whitespace-separated list of public keys.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "trusted-substituters",
        aliases: &["trusted-binary-caches"],
        description: "A list of Nix store URLs, separated by whitespace. These are not used by default, but users of the Nix daemon can enable them by specifying substituters.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "trusted-users",
        aliases: &[],
        description: "A list of user names, separated by whitespace. These users will have additional rights when connecting to the Nix daemon, such as the ability to specify additional substituters, or to import unsigned realisations or unsigned input-addressed store objects.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "upgrade-nix-store-path-url",
        aliases: &[],
        description: "Used by nix upgrade-nix, the URL of the file that contains the store paths of the latest Nix release.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "use-case-hack",
        aliases: &[],
        description: "Whether to enable a macOS-specific hack for dealing with file name case collisions.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "use-cgroups",
        aliases: &[],
        description: "Whether to execute builds inside cgroups. This is only supported on Linux. Cgroups are required and enabled automatically for derivations that require the uid-range system feature.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "use-registries",
        aliases: &[],
        description: "Whether to use flake registries to resolve flake references.",
        experimental_feature: Some("flakes"),
    },
    SettingMeta {
        key: "use-sqlite-wal",
        aliases: &[],
        description: "Whether SQLite should use WAL mode.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "use-xdg-base-directories",
        aliases: &[],
        description: "If set to true, Nix conforms to the XDG Base Directory Specification for files in $HOME.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "user-agent-suffix",
        aliases: &[],
        description: "String appended to the user agent in HTTP requests.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "warn-dirty",
        aliases: &[],
        description: "Whether to warn about dirty Git/Mercurial trees.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "warn-large-path-threshold",
        aliases: &[],
        description: "Warn when copying a path larger than this number of bytes to the Nix store (as determined by its NAR serialisation). Default is 0, which disables the warning. Set it to 1 to warn on all paths.",
        experimental_feature: None,
    },
    SettingMeta {
        key: "warn-short-path-literals",
        aliases: &[],
        description: "Deprecated. Use lint-short-path-literals = warn instead.",
        experimental_feature: None,
    },
];

/// Look up a setting by key or alias, case-insensitive.
#[must_use]
pub fn find_setting(name: &str) -> Option<&'static SettingMeta> {
    let lower = name.to_lowercase();
    KNOWN_SETTINGS
        .iter()
        .find(|s| s.key == lower || s.aliases.iter().any(|a| *a == lower))
}

#[cfg(test)]
mod tests {
    use super::{KNOWN_SETTINGS, find_setting};

    #[test]
    fn registry_is_sorted_by_key() {
        for pair in KNOWN_SETTINGS.windows(2) {
            assert!(
                pair[0].key < pair[1].key,
                "out of order: {:?} >= {:?}",
                pair[0].key,
                pair[1].key
            );
        }
    }

    #[test]
    fn find_by_canonical_key() {
        let s = find_setting("connect-timeout").expect("known key");
        assert_eq!(s.key, "connect-timeout");
    }

    #[test]
    fn find_by_alias() {
        let s = find_setting("build-cores").expect("alias lookup");
        assert_eq!(s.key, "cores");
    }

    #[test]
    fn find_is_case_insensitive() {
        assert!(find_setting("Connect-Timeout").is_some());
    }

    #[test]
    fn find_returns_none_for_unknown() {
        assert!(find_setting("not-a-real-setting").is_none());
    }
}
