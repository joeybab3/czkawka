use std::collections::BTreeMap;
use std::fs;
use std::fs::Metadata;

use gtk::prelude::*;
use gtk::{Align, CheckButton, Dialog, ResponseType, TextView};

use crate::flg;

use crate::gui_structs::gui_data::GuiData;
use crate::help_functions::*;
use crate::localizer_core::generate_translation_hashmap;
use crate::notebook_enums::*;

// TODO add support for checking if really symlink doesn't point to correct directory/file

pub fn connect_button_delete(gui_data: &GuiData) {
    let buttons_delete = gui_data.bottom_buttons.buttons_delete.clone();

    let gui_data = gui_data.clone(); // TODO this maybe can be replaced, not sure if worth to do it

    buttons_delete.connect_clicked(move |_| {
        glib::MainContext::default().spawn_local(delete_things(gui_data.clone()));
    });
}

pub async fn delete_things(gui_data: GuiData) {
    // validate_notebook_data(&gui_data);

    let notebook_main = gui_data.main_notebook.notebook_main.clone();
    let window_main = gui_data.window_main.clone();
    let check_button_settings_confirm_deletion = gui_data.settings.check_button_settings_confirm_deletion.clone();
    let check_button_settings_confirm_group_deletion = gui_data.settings.check_button_settings_confirm_group_deletion.clone();
    let image_preview_similar_images = gui_data.main_notebook.image_preview_similar_images.clone();
    let image_preview_duplicates = gui_data.main_notebook.image_preview_duplicates.clone();

    let main_tree_views = gui_data.main_notebook.get_main_tree_views();

    let check_button_settings_use_trash = gui_data.settings.check_button_settings_use_trash.clone();

    let preview_path = gui_data.preview_path.clone();

    let text_view_errors = gui_data.text_view_errors.clone();

    let nb_number = notebook_main.current_page().unwrap();
    let tree_view = &main_tree_views[nb_number as usize];
    let nb_object = &NOTEBOOKS_INFOS[nb_number as usize];

    let (number_of_selected_items, number_of_selected_groups) = check_how_much_elements_is_selected(tree_view, nb_object.column_color, nb_object.column_selection);

    // Nothing is selected
    if number_of_selected_items == 0 {
        return;
    }

    if !check_if_can_delete_files(&check_button_settings_confirm_deletion, &window_main, number_of_selected_items, number_of_selected_groups).await {
        return;
    }

    if let Some(column_color) = nb_object.column_color {
        if !check_button_settings_confirm_group_deletion.is_active()
            || !check_if_deleting_all_files_in_group(
                tree_view,
                column_color,
                nb_object.column_selection,
                nb_object.column_path,
                &window_main,
                &check_button_settings_confirm_group_deletion,
            )
            .await
        {
            tree_remove(
                tree_view,
                nb_object.column_name,
                nb_object.column_path,
                column_color,
                nb_object.column_selection,
                &check_button_settings_use_trash,
                &text_view_errors,
            );
        }
    } else {
        if nb_number == NotebookMainEnum::EmptyDirectories as u32 {
            empty_folder_remover(
                tree_view,
                nb_object.column_name,
                nb_object.column_path,
                nb_object.column_selection,
                &check_button_settings_use_trash,
                &text_view_errors,
            );
        } else {
            basic_remove(
                tree_view,
                nb_object.column_name,
                nb_object.column_path,
                nb_object.column_selection,
                &check_button_settings_use_trash,
                &text_view_errors,
            );
        }
    }

    match &nb_object.notebook_type {
        NotebookMainEnum::SimilarImages | NotebookMainEnum::Duplicate => {
            if nb_object.notebook_type == NotebookMainEnum::SimilarImages {
                image_preview_similar_images.hide();
            } else {
                image_preview_duplicates.hide();
            }
            *preview_path.borrow_mut() = "".to_string();
        }
        _ => {}
    }
}

pub async fn check_if_can_delete_files(
    check_button_settings_confirm_deletion: &gtk::CheckButton,
    window_main: &gtk::Window,
    number_of_selected_items: u64,
    number_of_selected_groups: u64,
) -> bool {
    if check_button_settings_confirm_deletion.is_active() {
        let (confirmation_dialog_delete, check_button) = create_dialog_ask_for_deletion(window_main, number_of_selected_items, number_of_selected_groups);

        let response_type = confirmation_dialog_delete.run_future().await;
        if response_type == gtk::ResponseType::Ok {
            if !check_button.is_active() {
                check_button_settings_confirm_deletion.set_active(false);
            }
            confirmation_dialog_delete.hide();
            confirmation_dialog_delete.close();
        } else {
            confirmation_dialog_delete.hide();
            confirmation_dialog_delete.close();
            return false;
        };
    }
    true
}

fn create_dialog_ask_for_deletion(window_main: &gtk::Window, number_of_selected_items: u64, number_of_selected_groups: u64) -> (Dialog, CheckButton) {
    let dialog = gtk::Dialog::builder().title(&flg!("delete_title_dialog")).transient_for(window_main).modal(true).build();
    let button_ok = dialog.add_button(&flg!("general_ok_button"), ResponseType::Ok);
    dialog.add_button(&flg!("general_close_button"), ResponseType::Cancel);

    let label: gtk::Label = gtk::Label::new(Some(&flg!("delete_question_label")));
    let label2: gtk::Label = match number_of_selected_groups {
        0 => gtk::Label::new(Some(&flg!(
            "delete_items_label",
            generate_translation_hashmap(vec![("items", number_of_selected_items.to_string())])
        ))),
        _ => gtk::Label::new(Some(&flg!(
            "delete_items_groups_label",
            generate_translation_hashmap(vec![("items", number_of_selected_items.to_string()), ("groups", number_of_selected_groups.to_string())])
        ))),
    };
    let check_button: gtk::CheckButton = gtk::CheckButton::with_label(&flg!("dialogs_ask_next_time"));
    check_button.set_active(true);
    check_button.set_halign(Align::Center);

    button_ok.grab_focus();

    let internal_box = get_dialog_box_child(&dialog);
    internal_box.add(&label);
    internal_box.add(&label2);
    internal_box.add(&check_button);
    internal_box.set_margin(5);
    check_button.set_margin_top(5);

    dialog.show_all();
    (dialog, check_button)
}

fn create_dialog_group_deletion(window_main: &gtk::Window) -> (Dialog, CheckButton) {
    let dialog = gtk::Dialog::builder()
        .title(&flg!("delete_all_files_in_group_title"))
        .transient_for(window_main)
        .modal(true)
        .build();
    let button_ok = dialog.add_button(&flg!("general_ok_button"), ResponseType::Ok);
    dialog.add_button(&flg!("general_close_button"), ResponseType::Cancel);

    let label: gtk::Label = gtk::Label::new(Some(&flg!("delete_all_files_in_group_label1")));
    let label2: gtk::Label = gtk::Label::new(Some(&flg!("delete_all_files_in_group_label2")));
    let check_button: gtk::CheckButton = gtk::CheckButton::with_label(&flg!("dialogs_ask_next_time"));
    check_button.set_active(true);
    check_button.set_halign(Align::Center);

    button_ok.grab_focus();

    let internal_box = get_dialog_box_child(&dialog);
    internal_box.add(&label);
    internal_box.add(&label2);
    internal_box.add(&check_button);

    dialog.show_all();
    (dialog, check_button)
}

pub async fn check_if_deleting_all_files_in_group(
    tree_view: &gtk::TreeView,
    column_color: i32,
    column_selection: i32,
    column_path: i32,
    window_main: &gtk::Window,
    check_button_settings_confirm_group_deletion: &gtk::CheckButton,
) -> bool {
    let model = get_list_store(tree_view);

    let mut selected_all_records: bool = true;

    if let Some(iter) = model.iter_first() {
        assert_eq!(model.value(&iter, column_color).get::<String>().unwrap(), HEADER_ROW_COLOR); // First element should be header

        // It is safe to remove any number of files in reference mode
        if !model.value(&iter, column_path).get::<String>().unwrap().is_empty() {
            return false;
        }

        loop {
            if !model.iter_next(&iter) {
                break;
            }

            if model.value(&iter, column_color).get::<String>().unwrap() == HEADER_ROW_COLOR {
                if selected_all_records {
                    break;
                }
                selected_all_records = true;
            } else {
                if !model.value(&iter, column_selection).get::<bool>().unwrap() {
                    selected_all_records = false;
                }
            }
        }
    } else {
        return false;
    }

    if !selected_all_records {
        return false;
    } else {
        let (confirmation_dialog_group_delete, check_button) = create_dialog_group_deletion(window_main);

        let response_type = confirmation_dialog_group_delete.run_future().await;
        if response_type == gtk::ResponseType::Ok {
            if !check_button.is_active() {
                check_button_settings_confirm_group_deletion.set_active(false);
            }
        } else {
            confirmation_dialog_group_delete.hide();
            confirmation_dialog_group_delete.close();
            return true;
        }
        confirmation_dialog_group_delete.hide();
        confirmation_dialog_group_delete.close();
    }

    false
}

pub fn empty_folder_remover(
    tree_view: &gtk::TreeView,
    column_file_name: i32,
    column_path: i32,
    column_selection: i32,
    check_button_settings_use_trash: &CheckButton,
    text_view_errors: &TextView,
) {
    let use_trash = check_button_settings_use_trash.is_active();

    let model = get_list_store(tree_view);

    let mut selected_rows = Vec::new();

    if let Some(iter) = model.iter_first() {
        loop {
            if model.value(&iter, column_selection).get::<bool>().unwrap() {
                selected_rows.push(model.path(&iter).unwrap());
            }
            if !model.iter_next(&iter) {
                break;
            }
        }
    }

    if selected_rows.is_empty() {
        return; // No selected rows
    }

    let mut messages: String = "".to_string();

    // Must be deleted from end to start, because when deleting entries, TreePath(and also TreeIter) will points to invalid data
    for (counter, tree_path) in selected_rows.iter().rev().enumerate() {
        handle_gtk_pending_event_counter(counter);
        let iter = model.iter(tree_path).unwrap();

        let name = model.value(&iter, column_file_name).get::<String>().unwrap();
        let path = model.value(&iter, column_path).get::<String>().unwrap();

        // We must check if folder is really empty or contains only other empty folders
        let mut error_happened = false;
        let mut folders_to_check: Vec<String> = vec![get_full_name_from_path_name(&path, &name)];
        let mut current_folder: String;
        let mut next_folder: String;
        'dir: while !folders_to_check.is_empty() {
            current_folder = folders_to_check.pop().unwrap();
            let read_dir = match fs::read_dir(&current_folder) {
                Ok(t) => t,
                Err(_inspected) => {
                    error_happened = true;
                    break 'dir;
                }
            };

            for entry in read_dir {
                let entry_data = match entry {
                    Ok(t) => t,
                    Err(_inspected) => {
                        error_happened = true;
                        break 'dir;
                    }
                };
                let metadata: Metadata = match entry_data.metadata() {
                    Ok(t) => t,
                    Err(_inspected) => {
                        error_happened = true;
                        break 'dir;
                    }
                };
                if metadata.is_dir() {
                    next_folder = "".to_owned()
                        + &current_folder
                        + "/"
                        + match &entry_data.file_name().into_string() {
                            Ok(t) => t,
                            Err(_inspected) => {
                                error_happened = true;
                                break 'dir;
                            }
                        };
                    folders_to_check.push(next_folder.clone());
                } else {
                    error_happened = true;
                }
            }
        }

        if !error_happened {
            if !use_trash {
                match fs::remove_dir_all(get_full_name_from_path_name(&path, &name)) {
                    Ok(_) => {
                        model.remove(&iter);
                    }
                    Err(_inspected) => error_happened = true,
                }
            } else {
                match trash::delete(get_full_name_from_path_name(&path, &name)) {
                    Ok(_) => {
                        model.remove(&iter);
                    }
                    Err(_inspected) => error_happened = true,
                }
            }
        }
        if error_happened {
            messages += &flg!(
                "delete_folder_failed",
                generate_translation_hashmap(vec![("dir", get_full_name_from_path_name(&path, &name))])
            );
            messages += "\n";
        }
    }

    text_view_errors.buffer().unwrap().set_text(messages.as_str());
}

pub fn basic_remove(
    tree_view: &gtk::TreeView,
    column_file_name: i32,
    column_path: i32,
    column_selection: i32,
    check_button_settings_use_trash: &CheckButton,
    text_view_errors: &TextView,
) {
    let use_trash = check_button_settings_use_trash.is_active();

    let model = get_list_store(tree_view);

    let mut messages: String = "".to_string();

    let mut selected_rows = Vec::new();

    if let Some(iter) = model.iter_first() {
        loop {
            if model.value(&iter, column_selection).get::<bool>().unwrap() {
                selected_rows.push(model.path(&iter).unwrap());
            }

            if !model.iter_next(&iter) {
                break;
            }
        }
    }

    if selected_rows.is_empty() {
        return; // No selected rows
    }

    // Must be deleted from end to start, because when deleting entries, TreePath(and also TreeIter) will points to invalid data
    for (counter, tree_path) in selected_rows.iter().rev().enumerate() {
        handle_gtk_pending_event_counter(counter);
        let iter = model.iter(tree_path).unwrap();

        let name = model.value(&iter, column_file_name).get::<String>().unwrap();
        let path = model.value(&iter, column_path).get::<String>().unwrap();

        if !use_trash {
            match fs::remove_file(get_full_name_from_path_name(&path, &name)) {
                Ok(_) => {
                    model.remove(&iter);
                }

                Err(e) => {
                    messages += flg!(
                        "delete_file_failed",
                        generate_translation_hashmap(vec![("name", get_full_name_from_path_name(&path, &name)), ("reason", e.to_string())])
                    )
                    .as_str();
                    messages += "\n";
                }
            }
        } else {
            match trash::delete(get_full_name_from_path_name(&path, &name)) {
                Ok(_) => {
                    model.remove(&iter);
                }
                Err(e) => {
                    messages += flg!(
                        "delete_file_failed",
                        generate_translation_hashmap(vec![("name", get_full_name_from_path_name(&path, &name)), ("reason", e.to_string())])
                    )
                    .as_str();
                    messages += "\n";
                }
            }
        }
    }

    text_view_errors.buffer().unwrap().set_text(messages.as_str());
}

// Remove all occurrences - remove every element which have same path and name as even non selected ones
pub fn tree_remove(
    tree_view: &gtk::TreeView,
    column_file_name: i32,
    column_path: i32,
    column_color: i32,
    column_selection: i32,
    check_button_settings_use_trash: &CheckButton,
    text_view_errors: &TextView,
) {
    let use_trash = check_button_settings_use_trash.is_active();

    let model = get_list_store(tree_view);

    let mut messages: String = "".to_string();

    let mut vec_path_to_delete: Vec<(String, String)> = Vec::new();
    let mut map_with_path_to_delete: BTreeMap<String, Vec<String>> = Default::default(); // BTreeMap<Path,Vec<FileName>>

    let mut selected_rows = Vec::new();

    if let Some(iter) = model.iter_first() {
        loop {
            if model.value(&iter, column_selection).get::<bool>().unwrap() {
                if model.value(&iter, column_color).get::<String>().unwrap() == MAIN_ROW_COLOR {
                    selected_rows.push(model.path(&iter).unwrap());
                } else {
                    panic!("Header row shouldn't be selected, please report bug.");
                }
            }

            if !model.iter_next(&iter) {
                break;
            }
        }
    }

    if selected_rows.is_empty() {
        return; // No selected rows
    }

    // Save to variable paths of files, and remove it when not removing all occurrences.
    for tree_path in selected_rows.iter().rev() {
        let iter = model.iter(tree_path).unwrap();

        let file_name = model.value(&iter, column_file_name).get::<String>().unwrap();
        let path = model.value(&iter, column_path).get::<String>().unwrap();

        model.remove(&iter);

        map_with_path_to_delete.entry(path.clone()).or_insert_with(Vec::new);
        map_with_path_to_delete.get_mut(path.as_str()).unwrap().push(file_name);
    }

    // Delete duplicated entries, and remove real files
    let mut counter = 0_usize;
    for (path, mut vec_file_name) in map_with_path_to_delete {
        vec_file_name.sort();
        vec_file_name.dedup();
        for file_name in vec_file_name {
            handle_gtk_pending_event_counter(counter);
            counter += 1;
            if !use_trash {
                if let Err(e) = fs::remove_file(get_full_name_from_path_name(&path, &file_name)) {
                    messages += flg!(
                        "delete_file_failed",
                        generate_translation_hashmap(vec![("name", get_full_name_from_path_name(&path, &file_name)), ("reason", e.to_string())])
                    )
                    .as_str();
                    messages += "\n";
                }
            } else if let Err(e) = trash::delete(get_full_name_from_path_name(&path, &file_name)) {
                messages += flg!(
                    "delete_file_failed",
                    generate_translation_hashmap(vec![("name", get_full_name_from_path_name(&path, &file_name)), ("reason", e.to_string())])
                )
                .as_str();
                messages += "\n";
            }

            vec_path_to_delete.push((path.clone(), file_name.clone()));
        }
    }

    clean_invalid_headers(&model, column_color, column_path);

    text_view_errors.buffer().unwrap().set_text(messages.as_str());
}
