use super::*;

impl App {
    pub(crate) fn reduce_signing(&mut self, message: Msg) -> Task<Msg> {
        match message {
            Msg::OpenSignDialog => {
                if self.model.format != Format::Pdf || self.model.file_path.is_none() {
                    return Task::none();
                }
                self.model.signing.get_or_insert_with(|| {
                    crate::signing::SignDraft::new(crate::signing::installed_fonts())
                });
                let open = crate::overlay_focus::open(&mut self.model, model::Overlay::Sign);
                Task::batch([
                    open,
                    iced::widget::operation::focus(crate::view::sign_dialog::name_field_id()),
                ])
            }
            Msg::SignNameChanged(name) => {
                if let Some(draft) = self.model.signing.as_mut() {
                    draft.set_name(name);
                }
                Task::none()
            }
            Msg::SignFontSelected(family) => {
                if let Some(draft) = self.model.signing.as_mut() {
                    draft.choose_family(family);
                }
                Task::none()
            }
            Msg::ChooseSigningCertificate => {
                if let Some(window) = self.main_window {
                    return self.begin_parented_pick(
                        crate::picker::PickerTarget::SigningCertificate,
                        window,
                    );
                }
                self.open_pick_unparented(crate::picker::PickerTarget::SigningCertificate)
            }
            Msg::ClearSigningCertificate => {
                if let Some(draft) = self.model.signing.as_mut() {
                    draft.certificate = None;
                    draft.password.clear();
                    draft.error = None;
                }
                Task::none()
            }
            Msg::SignPasswordChanged(password) => {
                if let Some(draft) = self.model.signing.as_mut() {
                    draft.password = password;
                    draft.error = None;
                }
                Task::none()
            }
            Msg::ToggleSignPasswordReveal => {
                if let Some(draft) = self.model.signing.as_mut() {
                    draft.password_revealed = !draft.password_revealed;
                }
                Task::none()
            }
            Msg::CancelSigning => {
                self.model.signing = None;
                if self.model.overlay == Some(model::Overlay::Sign) {
                    return crate::overlay_focus::dismiss(
                        &mut self.model,
                        enzui_iced_core::modal::DismissCause::CloseButton,
                    );
                }
                Task::none()
            }
            Msg::PlaceSignature => {
                let Some(draft) = self.model.signing.as_mut() else {
                    return Task::none();
                };
                if !draft.ready() {
                    return Task::none();
                }
                // A wrong password is said in the dialog, before placing.
                if let Some(path) = draft.certificate.clone() {
                    let opened = std::fs::read(&path).ok().and_then(|der| {
                        crate::plumbing::pdf_sign::Signer::from_pkcs12(&der, &draft.password).ok()
                    });
                    if opened.is_none() {
                        draft.error = Some(
                            "The certificate could not be opened; check the file and its password."
                                .to_string(),
                        );
                        return Task::none();
                    }
                }
                draft.placing = true;
                draft.error = None;
                crate::overlay_focus::dismiss(
                    &mut self.model,
                    enzui_iced_core::modal::DismissCause::CloseButton,
                )
            }
            Msg::SignaturePointer {
                page,
                at,
                displayed,
            } => {
                if let Some(draft) = self.model.signing.as_mut().filter(|d| d.placing) {
                    draft.pointer = Some((page, at, displayed));
                }
                Task::none()
            }
            Msg::SignatureClicked(page) => self.place_signature(page),
            Msg::SignedCopyWritten(result) => {
                self.model.signing = None;
                match result {
                    Ok(path) => {
                        self.model.announce("Signed copy saved");
                        self.open(path)
                    }
                    Err(reason) => {
                        self.model
                            .refuse(format!("The PDF was not signed: {reason}"));
                        Task::none()
                    }
                }
            }
            _ => Task::none(),
        }
    }

    fn place_signature(&mut self, page: usize) -> Task<Msg> {
        if self.model.rotation != crate::viewport::PageRotation::NONE {
            self.model
                .refuse("Turn the page upright to place a signature.");
            return Task::none();
        }
        let page_pts = self
            .model
            .pdf_text
            .get(page)
            .map(|text| (text.width_pts, text.height_pts))
            .filter(|(w, h)| *w > 0.0 && *h > 0.0);
        let Some(draft) = self.model.signing.as_mut().filter(|d| d.placing) else {
            return Task::none();
        };
        let (Some((pointer_page, at, displayed)), Some(page_pts)) = (draft.pointer, page_pts)
        else {
            return Task::none();
        };
        if pointer_page != page {
            return Task::none();
        }
        draft.placed = Some(crate::signing::placement_for(
            page,
            at,
            displayed,
            page_pts,
            draft.preview_size,
        ));
        draft.placing = false;
        // Offered as the original's name with `_signed`, beside it.
        if let Some(source) = self.model.file_path.as_deref() {
            self.picker
                .suggest_save_as(crate::picker::beside_with_suffix(source, "_signed"));
        }
        if let Some(window) = self.main_window {
            return self.begin_parented_pick(crate::picker::PickerTarget::SaveSigned, window);
        }
        self.open_pick_unparented(crate::picker::PickerTarget::SaveSigned)
    }

    pub(crate) fn write_signed_copy(&mut self, destination: std::path::PathBuf) -> Task<Msg> {
        let (Some(source), Some(draft)) =
            (self.model.file_path.clone(), self.model.signing.as_ref())
        else {
            return Task::none();
        };
        let (Some(image), Some(placement)) = (draft.stamp_image(), draft.placed) else {
            return Task::none();
        };
        let certificate = match draft.certificate.as_ref() {
            Some(path) => match std::fs::read(path) {
                Ok(der) => Some((der, draft.password.clone())),
                Err(_) => {
                    self.model.signing = None;
                    self.model.refuse("The certificate could not be read.");
                    return Task::none();
                }
            },
            None => None,
        };
        self.dispatch_blocking(move || {
            crate::signing::write_signed_copy(source, destination, image, placement, certificate)
        })
        .map(|result| {
            Msg::SignedCopyWritten(result.unwrap_or_else(|_| Err("the writer stopped".to_string())))
        })
    }
}
