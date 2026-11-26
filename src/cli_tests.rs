use super::*;

#[test]
fn output_write_errors_propagate_without_panicking() {
    struct ClosedPipe;
    impl Write for ClosedPipe {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::from(io::ErrorKind::BrokenPipe))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    assert_eq!(
        write_document(&mut ClosedPipe, b"report")
            .unwrap_err()
            .kind(),
        io::ErrorKind::BrokenPipe
    );
}

fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
    Cli::try_parse_from(std::iter::once("taku").chain(args.iter().copied()))
}

fn assert_parses(args: &[&str], expected: bool) {
    assert_eq!(parse(args).is_ok(), expected, "taku {args:?}");
}

#[test]
fn exact_commands_require_a_complete_variadic_resource_path() {
    for command in ["add", "remove", "forget"] {
        for (path, expected) in [
            (&[][..], false),
            (&["es"][..], false),
            (&["es", "roles"][..], false),
            (&["es", "roles", "one", "two"][..], true),
        ] {
            let args = [std::slice::from_ref(&command), path].concat();
            assert_parses(&args, expected);
        }
    }
}

#[test]
fn partial_commands_accept_every_contiguous_path_prefix() {
    let path = ["es", "roles", "one", "two"];
    for command in ["list", "fetch", "status", "diff", "pull", "push"] {
        for length in [0, 1, 2, path.len()] {
            let args = [std::slice::from_ref(&command), &path[..length]].concat();
            assert_parses(&args, true);
        }
    }
}

#[test]
fn remote_list_requires_target_and_type_and_owns_untracked() {
    for (args, expected) in [
        (&["list", "--remote"][..], false),
        (&["list", "--remote", "es"][..], false),
        (&["list", "--remote", "es", "roles"][..], true),
        (&["list", "--untracked"][..], false),
    ] {
        assert_parses(args, expected);
    }
}

#[test]
fn namespace_and_removed_selector_flags_are_rejected_outside_the_new_grammar() {
    assert_parses(&["status", "--namespace", "default"], false);
    for removed in ["--target", "--type", "--id"] {
        assert_parses(&["add", removed, "value", "es", "roles", "one"], false);
    }
    for command in ["add", "rename"] {
        assert_parses(&["app", command], false);
    }
}

#[test]
fn root_and_command_specific_options_stay_separate() {
    for args in [
        &["validate", "--environment", "dev"][..],
        &["app", "--namespace", "default"][..],
        &["promote", "--set", "token=value"][..],
    ] {
        assert_parses(args, false);
    }
    assert_parses(
        &["fetch", "--environment", "dev", "--set", "token=value"],
        true,
    );
}

#[test]
fn remaining_commands_keep_their_concise_grammar() {
    assert_parses(&["install"], false);
    for args in [
        &["install", "elasticsearch", "kibana"][..],
        &["update"][..],
        &["context", "set", "prod"][..],
        &["validate"][..],
        &["promote", "--from", "dev", "--to", "prod"][..],
    ] {
        assert_parses(args, true);
    }
}
