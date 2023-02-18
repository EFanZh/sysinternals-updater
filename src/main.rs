#![warn(
    absolute_paths_not_starting_with_crate,
    explicit_outlives_requirements,
    let_underscore_drop,
    macro_use_extern_crate,
    meta_variable_misuse,
    missing_abi,
    missing_docs,
    noop_method_call,
    pointer_structural_match,
    single_use_lifetimes,
    trivial_casts,
    trivial_numeric_casts,
    unsafe_code,
    unsafe_op_in_unsafe_fn,
    unused_crate_dependencies,
    unused_extern_crates,
    unused_import_braces,
    unused_lifetimes,
    unused_macro_rules,
    unused_qualifications,
    unused_tuple_struct_fields,
    variant_size_differences,
    clippy::alloc_instead_of_core,
    clippy::allow_attributes_without_reason,
    clippy::as_ptr_cast_mut,
    clippy::branches_sharing_code,
    clippy::cargo_common_metadata,
    clippy::clone_on_ref_ptr,
    clippy::cognitive_complexity,
    clippy::create_dir,
    clippy::dbg_macro,
    clippy::debug_assert_with_mut_call,
    clippy::decimal_literal_representation,
    clippy::deref_by_slicing,
    clippy::derive_partial_eq_without_eq,
    clippy::empty_drop,
    clippy::empty_line_after_outer_attr,
    clippy::empty_structs_with_brackets,
    clippy::equatable_if_let,
    clippy::fallible_impl_from,
    clippy::filetype_is_file,
    clippy::float_cmp_const,
    clippy::format_push_string,
    clippy::get_unwrap,
    clippy::if_then_some_else_none,
    clippy::imprecise_flops,
    clippy::iter_on_empty_collections,
    clippy::iter_on_single_items,
    clippy::iter_with_drain,
    clippy::large_include_file,
    clippy::let_underscore_must_use,
    clippy::lossy_float_literal,
    clippy::manual_clamp,
    clippy::map_err_ignore,
    clippy::mixed_read_write_in_expression,
    clippy::multiple_inherent_impl,
    clippy::mutex_atomic,
    clippy::mutex_integer,
    clippy::needless_collect,
    clippy::negative_feature_names,
    clippy::non_send_fields_in_send_ty,
    clippy::nonstandard_macro_braces,
    clippy::option_if_let_else,
    clippy::or_fun_call,
    clippy::panic,
    clippy::panic_in_result_fn,
    clippy::partial_pub_fields,
    clippy::path_buf_push_overwrite,
    clippy::pedantic,
    clippy::print_stderr,
    clippy::print_stdout,
    clippy::rc_buffer,
    clippy::rc_mutex,
    clippy::redundant_feature_names,
    clippy::redundant_pub_crate,
    clippy::rest_pat_in_fully_bound_structs,
    clippy::same_name_method,
    clippy::self_named_module_files,
    clippy::significant_drop_in_scrutinee,
    clippy::string_lit_as_bytes,
    clippy::string_to_string,
    clippy::suboptimal_flops,
    clippy::suspicious_operation_groupings,
    clippy::todo,
    clippy::trailing_empty_array,
    clippy::trait_duplication_in_bounds,
    clippy::transmute_undefined_repr,
    clippy::trivial_regex,
    clippy::try_err,
    clippy::type_repetition_in_bounds,
    clippy::undocumented_unsafe_blocks,
    clippy::unimplemented,
    clippy::unnecessary_safety_comment,
    clippy::unnecessary_safety_doc,
    clippy::unnecessary_self_imports,
    clippy::unneeded_field_pattern,
    clippy::unused_peekable,
    clippy::unused_rounding,
    clippy::use_debug,
    clippy::use_self,
    clippy::useless_let_if_seq,
    clippy::verbose_file_reads,
    clippy::wildcard_dependencies
)]
#![allow(
    missing_docs,
    clippy::cargo_common_metadata,
    clippy::print_stdout,
    clippy::wildcard_dependencies
)]

use filetime::FileTime;
use futures::stream::{FuturesUnordered, StreamExt};
use std::ffi::OsStr;
use std::fs::{self, DirEntry, Metadata};
use std::io::ErrorKind;
use std::path::Path;
use std::{env, io};
use tokio::runtime::Builder;

const SOURCE: &str = r#"\\live.sysinternals.com\tools"#;

macro_rules! log {
    ($level:literal, $($args:tt)*) => {
        {
            ::std::println!("[{:5}] {}", $level, ::std::format_args!($($args)*));
            ::std::io::Write::flush(&mut ::std::io::stdout()).ok();
        }
    };
}

macro_rules! info {
    ($($args:tt)*) => {
        log!("Info", $($args)*)
    };
}

macro_rules! error {
    ($($args:tt)*) => {
        log!("Error", $($args)*)
    };
}

fn walk_dir(path: &Path, f: &mut impl FnMut(DirEntry, Metadata)) {
    if let Ok(read_dir) = fs::read_dir(path) {
        for entry in read_dir.filter_map(Result::ok) {
            if let Ok(metadata) = entry.metadata() {
                if let Ok(file_type) = entry.file_type() {
                    if file_type.is_dir() {
                        walk_dir(entry.path().as_path(), f);
                    } else {
                        f(entry, metadata);
                    }
                }
            }
        }
    }
}

fn needs_update(source_metadata: &Metadata, target_metadata: Option<&Metadata>) -> bool {
    if let Some(target_metadata) = target_metadata {
        if let Ok(source_modification_time) = source_metadata.modified() {
            if let Ok(target_modification_time) = target_metadata.modified() {
                return source_metadata.len() != target_metadata.len()
                    || source_modification_time != target_modification_time;
            }
        }
    }

    true
}

async fn sync_file(
    target_dir: &Path,
    dir_entry: DirEntry,
    source_metadata: Metadata,
) -> io::Result<(Box<Path>, bool)> {
    let source_path = dir_entry.path();
    let relative_path = source_path.strip_prefix(SOURCE).unwrap();
    let target_path = target_dir.join(relative_path);
    let target_metadata = tokio::fs::metadata(target_path.as_path()).await;

    if needs_update(&source_metadata, target_metadata.as_ref().ok()) {
        tokio::fs::create_dir_all(target_path.parent().unwrap()).await?;
        tokio::fs::copy(source_path.as_path(), target_path.as_path()).await?;

        filetime::set_file_mtime(
            target_path,
            FileTime::from_last_modification_time(&source_metadata),
        )?;

        Ok((relative_path.into(), true))
    } else {
        Ok((relative_path.into(), false))
    }
}

async fn main_async() -> io::Result<()> {
    if let Some(target_dir) = env::args_os().nth(1) {
        let target_dir = Path::new(&target_dir);
        let mut tasks = FuturesUnordered::new();

        info!("Checking updates...");

        walk_dir(SOURCE.as_ref(), &mut |dir_entry, source_metadata| {
            tasks.push(sync_file(target_dir, dir_entry, source_metadata));
        });

        let total = tasks.len();

        info!("Synchronizing {} files...", total);

        let mut finished = 0_usize;

        while let Some(task_result) = tasks.next().await {
            finished += 1;

            match task_result {
                Ok((path, downloaded)) => {
                    info!(
                        "[{:>3} / {}] {}: {}.",
                        finished,
                        total,
                        if downloaded {
                            "Downloaded"
                        } else {
                            "Up to date"
                        },
                        path.display(),
                    );
                }
                Err(error) => error!(
                    "[{:>3} / {}] Failed to download: {}",
                    finished, total, error
                ),
            }
        }
    } else {
        println!(
            "Usage: {} <TARGET_DIR>",
            env::current_exe()?
                .file_stem()
                .and_then(OsStr::to_str)
                .ok_or(ErrorKind::InvalidData)?,
        );
    }

    Ok(())
}

fn main() -> io::Result<()> {
    Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(main_async())
}
