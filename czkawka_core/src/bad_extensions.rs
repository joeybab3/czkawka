use std::collections::BTreeSet;
use std::fs::File;
use std::io::prelude::*;
use std::io::BufWriter;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::sleep;
use std::time::{Duration, SystemTime};
use std::{mem, thread};

use crossbeam_channel::Receiver;
use mime_guess::get_mime_extensions;
use rayon::prelude::*;

use crate::common::{Common, LOOP_DURATION};
use crate::common_dir_traversal::{CheckingMethod, DirTraversalBuilder, DirTraversalResult, FileEntry, ProgressData};
use crate::common_directory::Directories;
use crate::common_extensions::Extensions;
use crate::common_items::ExcludedItems;
use crate::common_messages::Messages;
use crate::common_traits::*;

// This adds several workarounds for bugs/invalid recognizing types by external libraries
// ("real_content_extension", "current_file_extension")
static WORKAROUNDS: &[(&str, &str)] = &[
    // Wine/Windows
    ("exe", "acm"),
    ("exe", "ax"),
    ("exe", "bck"),
    ("exe", "com"),
    ("exe", "cpl"),
    ("exe", "cpl"),
    ("exe", "dll"),
    ("exe", "dll16"),
    ("exe", "drv"),
    ("exe", "drv16"),
    ("exe", "ds"),
    ("exe", "exe16"),
    ("exe", "fon"), // Type of font or something else
    ("exe", "msstyles"),
    ("exe", "orig"),
    ("exe", "sys"),
    ("exe", "tlb"),
    ("exe", "vxd"),
    // Other
    ("exe", "doc"), // Not sure whe docx is not recognized
    ("exe", "sys"),
    ("gz", "blend"),
    ("gz", "crate"),
    ("gz", "svgz"),
    ("gz", "tgz"),
    ("html", "md"),
    ("html", "svg"), // Quite strange, but yes it works
    ("jpg", "jfif"),
    ("obj", "o"),
    ("odp", "otp"),
    ("odt", "ott"),
    ("ogg", "ogv"),
    ("pptx", "ppsx"),
    ("sh", "bash"),
    ("sh", "py"),
    ("sh", "rs"),
    ("xml", "cmb"),
    ("xml", "conf"),
    ("xml", "dae"),
    ("xml", "docbook"),
    ("xml", "gir"),
    ("xml", "glade"),
    ("xml", "html"),
    ("xml", "kdenlive"),
    ("xml", "lang"),
    ("xml", "svg"),
    ("xml", "ui"),
    ("xml", "vcproj"),
    ("zip", "apk"),
    ("zip", "dat"),
    ("zip", "jar"),
    ("zip", "kra"),
    ("zip", "nupkg"),
    ("zip", "pptx"),
    ("zip", "whl"),
    ("zip", "xpi"),
];

#[derive(Clone)]
pub struct BadFileEntry {
    pub path: PathBuf,
    pub modified_date: u64,
    pub size: u64,
    pub current_extension: String,
    pub proper_extensions: String,
}
/// Info struck with helpful information's about results
#[derive(Default)]
pub struct Info {
    pub number_of_files_with_bad_extension: usize,
}

impl Info {
    pub fn new() -> Self {
        Default::default()
    }
}

pub struct BadExtensions {
    text_messages: Messages,
    information: Info,
    files_to_check: Vec<FileEntry>,
    bad_extensions_files: Vec<BadFileEntry>,
    directories: Directories,
    allowed_extensions: Extensions,
    excluded_items: ExcludedItems,
    minimal_file_size: u64,
    maximal_file_size: u64,
    recursive_search: bool,
    stopped_search: bool,
    save_also_as_json: bool,
    include_files_without_extension: bool,
}

impl BadExtensions {
    pub fn new() -> Self {
        Self {
            text_messages: Messages::new(),
            information: Info::new(),
            recursive_search: true,
            allowed_extensions: Extensions::new(),
            directories: Directories::new(),
            excluded_items: ExcludedItems::new(),
            files_to_check: Default::default(),
            stopped_search: false,
            minimal_file_size: 8192,
            maximal_file_size: u64::MAX,
            bad_extensions_files: Default::default(),
            save_also_as_json: false,
            include_files_without_extension: true,
        }
    }

    pub fn find_bad_extensions_files(&mut self, stop_receiver: Option<&Receiver<()>>, progress_sender: Option<&futures::channel::mpsc::UnboundedSender<ProgressData>>) {
        self.directories.optimize_directories(self.recursive_search, &mut self.text_messages);
        if !self.check_files(stop_receiver, progress_sender) {
            self.stopped_search = true;
            return;
        }
        if !self.look_for_bad_extensions_files(stop_receiver, progress_sender) {
            self.stopped_search = true;
            return;
        }
        self.debug_print();
    }

    pub fn get_stopped_search(&self) -> bool {
        self.stopped_search
    }

    pub const fn get_bad_extensions_files(&self) -> &Vec<BadFileEntry> {
        &self.bad_extensions_files
    }

    pub fn set_maximal_file_size(&mut self, maximal_file_size: u64) {
        self.maximal_file_size = match maximal_file_size {
            0 => 1,
            t => t,
        };
    }
    pub fn set_minimal_file_size(&mut self, minimal_file_size: u64) {
        self.minimal_file_size = match minimal_file_size {
            0 => 1,
            t => t,
        };
    }

    pub const fn get_text_messages(&self) -> &Messages {
        &self.text_messages
    }

    pub const fn get_information(&self) -> &Info {
        &self.information
    }

    pub fn set_save_also_as_json(&mut self, save_also_as_json: bool) {
        self.save_also_as_json = save_also_as_json;
    }

    pub fn set_recursive_search(&mut self, recursive_search: bool) {
        self.recursive_search = recursive_search;
    }

    pub fn set_included_directory(&mut self, included_directory: Vec<PathBuf>) -> bool {
        self.directories.set_included_directory(included_directory, &mut self.text_messages)
    }

    pub fn set_excluded_directory(&mut self, excluded_directory: Vec<PathBuf>) {
        self.directories.set_excluded_directory(excluded_directory, &mut self.text_messages);
    }
    pub fn set_allowed_extensions(&mut self, allowed_extensions: String) {
        self.allowed_extensions.set_allowed_extensions(allowed_extensions, &mut self.text_messages);
    }

    pub fn set_excluded_items(&mut self, excluded_items: Vec<String>) {
        self.excluded_items.set_excluded_items(excluded_items, &mut self.text_messages);
    }

    fn check_files(&mut self, stop_receiver: Option<&Receiver<()>>, progress_sender: Option<&futures::channel::mpsc::UnboundedSender<ProgressData>>) -> bool {
        let result = DirTraversalBuilder::new()
            .root_dirs(self.directories.included_directories.clone())
            .group_by(|_fe| ())
            .stop_receiver(stop_receiver)
            .progress_sender(progress_sender)
            .minimal_file_size(self.minimal_file_size)
            .maximal_file_size(self.maximal_file_size)
            .directories(self.directories.clone())
            .allowed_extensions(self.allowed_extensions.clone())
            .excluded_items(self.excluded_items.clone())
            .recursive_search(self.recursive_search)
            .build()
            .run();
        match result {
            DirTraversalResult::SuccessFiles {
                start_time,
                grouped_file_entries,
                warnings,
            } => {
                if let Some(files_to_check) = grouped_file_entries.get(&()) {
                    self.files_to_check = files_to_check.clone();
                }
                self.text_messages.warnings.extend(warnings);
                Common::print_time(start_time, SystemTime::now(), "check_files".to_string());
                true
            }
            DirTraversalResult::SuccessFolders { .. } => {
                unreachable!()
            }
            DirTraversalResult::Stopped => false,
        }
    }

    fn look_for_bad_extensions_files(&mut self, stop_receiver: Option<&Receiver<()>>, progress_sender: Option<&futures::channel::mpsc::UnboundedSender<ProgressData>>) -> bool {
        let system_time = SystemTime::now();

        let include_files_without_extension = self.include_files_without_extension;

        let check_was_breaked = AtomicBool::new(false); // Used for breaking from GUI and ending check thread

        //// PROGRESS THREAD START
        let progress_thread_run = Arc::new(AtomicBool::new(true));
        let atomic_file_counter = Arc::new(AtomicUsize::new(0));

        let progress_thread_handle = if let Some(progress_sender) = progress_sender {
            let progress_send = progress_sender.clone();
            let progress_thread_run = progress_thread_run.clone();
            let atomic_file_counter = atomic_file_counter.clone();
            let entries_to_check = self.files_to_check.len();
            thread::spawn(move || loop {
                progress_send
                    .unbounded_send(ProgressData {
                        checking_method: CheckingMethod::None,
                        current_stage: 1,
                        max_stage: 1,
                        entries_checked: atomic_file_counter.load(Ordering::Relaxed) as usize,
                        entries_to_check,
                    })
                    .unwrap();
                if !progress_thread_run.load(Ordering::Relaxed) {
                    break;
                }
                sleep(Duration::from_millis(LOOP_DURATION as u64));
            })
        } else {
            thread::spawn(|| {})
        };

        let mut files_to_check = Default::default();
        mem::swap(&mut files_to_check, &mut self.files_to_check);

        //// PROGRESS THREAD END
        self.bad_extensions_files = files_to_check
            .into_par_iter() // TODO into par iter after
            .map(|file_entry| {
                atomic_file_counter.fetch_add(1, Ordering::Relaxed);
                if stop_receiver.is_some() && stop_receiver.unwrap().try_recv().is_ok() {
                    check_was_breaked.store(true, Ordering::Relaxed);
                    return None;
                }

                let current_extension;

                // Check what exactly content file contains
                let kind = match infer::get_from_path(&file_entry.path) {
                    Ok(k) => match k {
                        Some(t) => t,
                        None => return Some(None),
                    },
                    Err(_) => return Some(None),
                };
                let proper_extension = kind.extension();

                // Extract current extension from file
                if let Some(extension) = file_entry.path.extension() {
                    let extension = extension.to_string_lossy().to_lowercase();
                    // Text longer than 10 characters is not considered as extension
                    if extension.len() > 10 {
                        current_extension = "".to_string();
                    } else {
                        current_extension = extension;
                    }
                } else {
                    current_extension = "".to_string();
                }

                // Already have proper extension, no need to do more things
                if current_extension == proper_extension {
                    return Some(None);
                }

                // Check for all extensions that file can use(not sure if it is worth to do it)
                let mut all_available_extensions: BTreeSet<_> = Default::default();
                let think_extension = match current_extension.is_empty() {
                    true => "".to_string(),
                    false => {
                        for mim in mime_guess::from_ext(proper_extension) {
                            if let Some(all_ext) = get_mime_extensions(&mim) {
                                for ext in all_ext {
                                    all_available_extensions.insert(ext);
                                }
                            }
                        }

                        // Workarounds
                        for (pre, post) in WORKAROUNDS {
                            if post == &current_extension.as_str() && all_available_extensions.contains(&pre) {
                                all_available_extensions.insert(post);
                            }
                        }

                        let mut guessed_multiple_extensions = "".to_string();
                        for ext in &all_available_extensions {
                            guessed_multiple_extensions.push_str(ext);
                            guessed_multiple_extensions.push(',');
                        }
                        guessed_multiple_extensions.pop();

                        guessed_multiple_extensions
                    }
                };

                if all_available_extensions.is_empty() {
                    // Not found any extension
                    return Some(None);
                } else if current_extension.is_empty() {
                    if !include_files_without_extension {
                        println!("Empty extension which is disabled by settings");
                        return Some(None);
                    }
                } else if all_available_extensions.take(&current_extension.as_str()).is_some() {
                    // Found proper extension
                    return Some(None);
                }

                Some(Some(BadFileEntry {
                    path: file_entry.path,
                    modified_date: file_entry.modified_date,
                    size: file_entry.size,
                    current_extension,
                    proper_extensions: think_extension,
                }))
            })
            .while_some()
            .filter(|file_entry| file_entry.is_some())
            .map(|file_entry| file_entry.unwrap())
            .collect::<Vec<_>>();

        // End thread which send info to gui
        progress_thread_run.store(false, Ordering::Relaxed);
        progress_thread_handle.join().unwrap();

        // Break if stop was clicked
        if check_was_breaked.load(Ordering::Relaxed) {
            return false;
        }

        self.information.number_of_files_with_bad_extension = self.bad_extensions_files.len();

        Common::print_time(system_time, SystemTime::now(), "sort_images - reading data from files in parallel".to_string());

        // Clean unused data
        self.files_to_check = Default::default();

        true
    }
}

impl Default for BadExtensions {
    fn default() -> Self {
        Self::new()
    }
}

impl DebugPrint for BadExtensions {
    #[allow(dead_code)]
    #[allow(unreachable_code)]
    /// Debugging printing - only available on debug build
    fn debug_print(&self) {
        #[cfg(not(debug_assertions))]
        {
            return;
        }
        println!("---------------DEBUG PRINT---------------");
        println!("### Information's");

        println!("Errors size - {}", self.text_messages.errors.len());
        println!("Warnings size - {}", self.text_messages.warnings.len());
        println!("Messages size - {}", self.text_messages.messages.len());

        println!("### Other");

        println!("Excluded items - {:?}", self.excluded_items.items);
        println!("Included directories - {:?}", self.directories.included_directories);
        println!("Excluded directories - {:?}", self.directories.excluded_directories);
        println!("Recursive search - {}", self.recursive_search);
        println!("-----------------------------------------");
    }
}

impl SaveResults for BadExtensions {
    fn save_results_to_file(&mut self, file_name: &str) -> bool {
        let start_time: SystemTime = SystemTime::now();
        let file_name: String = match file_name {
            "" => "results.txt".to_string(),
            k => k.to_string(),
        };

        let file_handler = match File::create(&file_name) {
            Ok(t) => t,
            Err(e) => {
                self.text_messages.errors.push(format!("Failed to create file {}, reason {}", file_name, e));
                return false;
            }
        };
        let mut writer = BufWriter::new(file_handler);

        if let Err(e) = writeln!(
            writer,
            "Results of searching {:?} with excluded directories {:?} and excluded items {:?}",
            self.directories.included_directories, self.directories.excluded_directories, self.excluded_items.items
        ) {
            self.text_messages.errors.push(format!("Failed to save results to file {}, reason {}", file_name, e));
            return false;
        }

        if !self.bad_extensions_files.is_empty() {
            writeln!(writer, "Found {} files with invalid extension.", self.information.number_of_files_with_bad_extension).unwrap();
            for file_entry in self.bad_extensions_files.iter() {
                writeln!(writer, "{}", file_entry.path.display()).unwrap();
            }
        } else {
            write!(writer, "Not found any files with invalid extension.").unwrap();
        }
        Common::print_time(start_time, SystemTime::now(), "save_results_to_file".to_string());
        true
    }
}

impl PrintResults for BadExtensions {
    /// Print information's about duplicated entries
    /// Only needed for CLI
    fn print_results(&self) {
        let start_time: SystemTime = SystemTime::now();
        println!("Found {} files with invalid extension.\n", self.information.number_of_files_with_bad_extension);
        for file_entry in self.bad_extensions_files.iter() {
            println!("{}", file_entry.path.display());
        }

        Common::print_time(start_time, SystemTime::now(), "print_entries".to_string());
    }
}
