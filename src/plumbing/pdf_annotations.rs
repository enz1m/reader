//! Reconciles native PDF annotations with durable sidecar records.
//!
//! Only annotations whose native identity, page content, text, and geometry
//! still agree are projected onto the reading surface.

use std::collections::{HashMap, HashSet};

use crate::model::PdfAnnotationState;
use crate::pdf::PdfPageText;
use crate::plumbing::pdf_ops::{
    PdfAnnotationKind, PdfAnnotationSnapshot, PdfPageIdentity, PdfRect, StoredAnnotation,
    text_fingerprint,
};
use crate::plumbing::persistence::{
    Annotation, AnnotationPosition, AnnotationsFile, PdfAnnotationBinding, add_annotation_full,
};

pub struct Reconciliation {
    pub states: HashMap<u64, PdfAnnotationState>,
    pub changed: bool,
}

pub fn normalized_position(
    page: usize,
    rect: PdfRect,
    identity: &PdfPageIdentity,
) -> Option<AnnotationPosition> {
    if !rect.x0.is_finite()
        || !rect.x1.is_finite()
        || !rect.y0.is_finite()
        || !rect.y1.is_finite()
        || identity.width <= 0.0
        || identity.height <= 0.0
        || identity.rotation_degrees.rem_euclid(360) != 0
    {
        return None;
    }
    let x0 = rect.x0.min(rect.x1);
    let x1 = rect.x0.max(rect.x1);
    let y0 = rect.y0.min(rect.y1);
    let y1 = rect.y0.max(rect.y1);
    let x = ((x0 - identity.x0) / identity.width).clamp(0.0, 1.0);
    let y = ((identity.y0 + identity.height - y1) / identity.height).clamp(0.0, 1.0);
    let w = ((x1 - x0) / identity.width).clamp(0.0, 1.0 - x);
    let h = ((y1 - y0) / identity.height).clamp(0.0, 1.0 - y);
    (w > 0.0 && h > 0.0).then_some(AnnotationPosition::PdfRect { page, x, y, w, h })
}

pub fn absolute_rect(annotation: &Annotation, pages: &[PdfPageIdentity]) -> Option<PdfRect> {
    let AnnotationPosition::PdfRect { page, x, y, w, h } =
        AnnotationPosition::parse(&annotation.position, annotation.page)
    else {
        return None;
    };
    let identity = pages.get(page)?;
    if identity.rotation_degrees.rem_euclid(360) != 0 {
        return None;
    }
    Some(PdfRect::new(
        identity.x0 + x * identity.width,
        identity.y0 + (1.0 - y - h) * identity.height,
        identity.x0 + (x + w) * identity.width,
        identity.y0 + (1.0 - y) * identity.height,
    ))
}

pub fn reconcile(
    file: &mut AnnotationsFile,
    snapshot: &PdfAnnotationSnapshot,
    page_text: &[PdfPageText],
) -> Reconciliation {
    let mut states = HashMap::new();
    let mut changed = false;
    let native_by_id: HashMap<&str, &StoredAnnotation> = snapshot
        .annotations
        .iter()
        .filter(|annotation| supported(annotation))
        .map(|annotation| (annotation.native_id.as_str(), annotation))
        .collect();
    let mut seen_ids = HashSet::new();
    let duplicate_ids: HashSet<&str> = snapshot
        .annotations
        .iter()
        .filter(|annotation| supported(annotation))
        .filter_map(|annotation| {
            (!seen_ids.insert(annotation.native_id.as_str()))
                .then_some(annotation.native_id.as_str())
        })
        .collect();
    let mut matched = HashSet::new();

    for annotation in &mut file.annotations {
        let Some(binding) = annotation.pdf_binding.clone() else {
            let candidate = snapshot.annotations.iter().find(|native| {
                !matched.contains(native.native_id.as_str())
                    && !duplicate_ids.contains(native.native_id.as_str())
                    && legacy_matches(annotation, native, &snapshot.pages, page_text)
            });
            if let Some(native) = candidate {
                annotation.pdf_binding = Some(binding_for(
                    native,
                    snapshot,
                    if annotation.kind == "highlight" {
                        &annotation.text
                    } else {
                        ""
                    },
                ));
                matched.insert(native.native_id.clone());
                states.insert(
                    annotation.id,
                    state_for(annotation, native, snapshot, page_text),
                );
                changed = true;
            } else if matches!(
                AnnotationPosition::parse(&annotation.position, annotation.page),
                AnnotationPosition::PdfRect { .. }
            ) {
                let state = absolute_rect(annotation, &snapshot.pages)
                    .and_then(|rect| text_for_page_rect(annotation.page, rect, page_text))
                    .filter(|text| text == &annotation.text)
                    .map(|_| PdfAnnotationState::Verified)
                    .unwrap_or(PdfAnnotationState::Unverified);
                states.insert(annotation.id, state);
            }
            continue;
        };

        if native_by_id.contains_key(binding.native_id.as_str()) {
            matched.insert(binding.native_id.clone());
        }
        if duplicate_ids.contains(binding.native_id.as_str()) {
            states.insert(annotation.id, PdfAnnotationState::Conflict);
            continue;
        }
        let Some(page_identity) = snapshot.pages.get(annotation.page) else {
            states.insert(annotation.id, PdfAnnotationState::Orphaned);
            continue;
        };
        if page_identity.fingerprint != binding.page_fingerprint {
            states.insert(annotation.id, PdfAnnotationState::Orphaned);
            continue;
        }
        let Some(native) = native_by_id.get(binding.native_id.as_str()).copied() else {
            states.insert(annotation.id, PdfAnnotationState::Conflict);
            continue;
        };
        matched.insert(native.native_id.clone());
        if native.page != annotation.page || native.fingerprint != binding.native_fingerprint {
            states.insert(annotation.id, PdfAnnotationState::Conflict);
            continue;
        }
        if annotation.kind == "highlight"
            && binding.text_fingerprint.is_empty()
            && let Some(text) = text_for_rect(native, page_text)
        {
            annotation.text = text.clone();
            if annotation.note == text {
                annotation.note.clear();
            }
            if let Some(binding) = annotation.pdf_binding.as_mut() {
                binding.text_fingerprint = text_fingerprint(&text);
            }
            changed = true;
        }
        states.insert(
            annotation.id,
            state_for(annotation, native, snapshot, page_text),
        );
    }

    for native in &snapshot.annotations {
        if !supported(native) || matched.contains(native.native_id.as_str()) {
            continue;
        }
        if duplicate_ids.contains(native.native_id.as_str()) {
            let id = import_native(file, native, snapshot, None, String::new());
            states.insert(id, PdfAnnotationState::Conflict);
            matched.insert(native.native_id.clone());
            changed = true;
            continue;
        }
        let Some(rect) = native.rect else {
            let id = import_native(file, native, snapshot, None, String::new());
            states.insert(id, PdfAnnotationState::Unverified);
            changed = true;
            continue;
        };
        let Some(identity) = snapshot.pages.get(native.page) else {
            continue;
        };
        let position = normalized_position(native.page, rect, identity);
        let selected_text = if native.subtype == "Highlight" {
            text_for_rect(native, page_text).unwrap_or_default()
        } else {
            String::new()
        };
        let id = import_native(file, native, snapshot, position, selected_text.clone());
        let state = if native.subtype == "Text" || !selected_text.is_empty() {
            PdfAnnotationState::Verified
        } else {
            PdfAnnotationState::Unverified
        };
        states.insert(id, state);
        changed = true;
    }

    Reconciliation { states, changed }
}

fn supported(annotation: &StoredAnnotation) -> bool {
    matches!(annotation.subtype.as_str(), "Highlight" | "Text")
}

fn import_native(
    file: &mut AnnotationsFile,
    native: &StoredAnnotation,
    snapshot: &PdfAnnotationSnapshot,
    position: Option<AnnotationPosition>,
    selected_text: String,
) -> u64 {
    let highlight = native.subtype == "Highlight";
    let text = if highlight {
        if selected_text.is_empty() {
            "Highlighted text unavailable".to_string()
        } else {
            selected_text.clone()
        }
    } else {
        native.contents.clone()
    };
    let note = if highlight && native.contents != selected_text {
        native.contents.as_str()
    } else {
        ""
    };
    let id = add_annotation_full(
        file,
        native.page,
        if highlight { "highlight" } else { "note" },
        &text,
        if highlight && !native.color.is_empty() {
            &native.color
        } else if highlight {
            "yellow"
        } else {
            ""
        },
        position.unwrap_or(AnnotationPosition::None),
        if highlight { "highlight" } else { "" },
        note,
    );
    if let Some(annotation) = file
        .annotations
        .iter_mut()
        .find(|annotation| annotation.id == id)
    {
        annotation.pdf_binding = Some(binding_for(native, snapshot, &selected_text));
    }
    id
}

fn binding_for(
    native: &StoredAnnotation,
    snapshot: &PdfAnnotationSnapshot,
    selected_text: &str,
) -> PdfAnnotationBinding {
    PdfAnnotationBinding {
        native_id: native.native_id.clone(),
        document_fingerprint: snapshot.document_fingerprint.clone(),
        page_fingerprint: snapshot
            .pages
            .get(native.page)
            .map(|page| page.fingerprint.clone())
            .unwrap_or_default(),
        native_fingerprint: native.fingerprint.clone(),
        text_fingerprint: if selected_text.is_empty() {
            String::new()
        } else {
            text_fingerprint(selected_text)
        },
    }
}

fn state_for(
    annotation: &Annotation,
    native: &StoredAnnotation,
    snapshot: &PdfAnnotationSnapshot,
    page_text: &[PdfPageText],
) -> PdfAnnotationState {
    let Some(binding) = annotation.pdf_binding.as_ref() else {
        return PdfAnnotationState::Unverified;
    };
    let Some(sidecar_rect) = absolute_rect(annotation, &snapshot.pages) else {
        return PdfAnnotationState::Unverified;
    };
    let Some(native_rect) = native.rect else {
        return PdfAnnotationState::Unverified;
    };
    if !rects_match(sidecar_rect, native_rect) {
        return PdfAnnotationState::Conflict;
    }
    if annotation.kind == "highlight" {
        let native_color = if native.color.is_empty() {
            "yellow"
        } else {
            native.color.as_str()
        };
        let legacy_selected_contents =
            annotation.note.is_empty() && native.contents == annotation.text;
        if (!legacy_selected_contents && native.contents != annotation.note)
            || native_color != annotation.color
        {
            return PdfAnnotationState::Conflict;
        }
        if binding.text_fingerprint.is_empty() {
            return PdfAnnotationState::Unverified;
        }
        if binding.text_fingerprint != text_fingerprint(&annotation.text) {
            return PdfAnnotationState::Conflict;
        }
        let Some(current_text) = text_for_rect(native, page_text) else {
            return PdfAnnotationState::Unverified;
        };
        if binding.text_fingerprint != text_fingerprint(&current_text) {
            return PdfAnnotationState::Orphaned;
        }
    } else if native.contents != annotation.text {
        return PdfAnnotationState::Conflict;
    }
    if native.rect.is_none() {
        return PdfAnnotationState::Unverified;
    }
    if binding.document_fingerprint == snapshot.document_fingerprint {
        PdfAnnotationState::Verified
    } else {
        PdfAnnotationState::VerifiedAfterDocumentChange
    }
}

fn legacy_matches(
    annotation: &Annotation,
    native: &StoredAnnotation,
    pages: &[PdfPageIdentity],
    page_text: &[PdfPageText],
) -> bool {
    if annotation.page != native.page {
        return false;
    }
    let kind_matches = (annotation.kind == "highlight" && native.subtype == "Highlight")
        || (annotation.kind == "note" && native.subtype == "Text");
    if !kind_matches {
        return false;
    }
    let Some(native_rect) = native.rect else {
        return false;
    };
    let Some(sidecar_rect) = absolute_rect(annotation, pages) else {
        return false;
    };
    let geometry_matches = rects_match(native_rect, sidecar_rect);
    if !geometry_matches {
        return false;
    }
    if annotation.kind == "highlight" {
        text_for_rect(native, page_text).as_deref() == Some(annotation.text.as_str())
            && (native.contents.is_empty()
                || native.contents == annotation.text
                || native.contents == annotation.note)
    } else {
        native.contents == annotation.text
    }
}

pub(crate) fn rects_match(left: PdfRect, right: PdfRect) -> bool {
    let close = |a: f32, b: f32| (a - b).abs() <= 0.75;
    close(left.x0, right.x0)
        && close(left.y0, right.y0)
        && close(left.x1, right.x1)
        && close(left.y1, right.y1)
}

fn text_for_page_rect(page: usize, rect: PdfRect, pages: &[PdfPageText]) -> Option<String> {
    let page = pages.get(page)?;
    let mut text = String::new();
    for line in &page.lines {
        let intersects = line.rect[0] < rect.x1
            && line.rect[2] > rect.x0
            && line.rect[1] < rect.y1
            && line.rect[3] > rect.y0;
        if intersects {
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(&line.text);
        }
    }
    (!text.is_empty()).then_some(text)
}

fn text_for_rect(native: &StoredAnnotation, pages: &[PdfPageText]) -> Option<String> {
    text_for_page_rect(native.page, native.rect?, pages)
}

pub fn kind_for(annotation: &Annotation) -> PdfAnnotationKind {
    if annotation.kind == "highlight" {
        PdfAnnotationKind::Highlight
    } else {
        PdfAnnotationKind::Comment
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdf::PdfTextLine;
    use crate::plumbing::pdf_ops::annotation_fingerprint;

    fn page(fingerprint: &str) -> PdfPageIdentity {
        PdfPageIdentity {
            fingerprint: fingerprint.to_string(),
            rotation_degrees: 0,
            x0: 0.0,
            y0: 0.0,
            width: 600.0,
            height: 800.0,
        }
    }

    fn native(id: &str, contents: &str) -> StoredAnnotation {
        let rect = PdfRect::new(60.0, 680.0, 300.0, 700.0);
        StoredAnnotation {
            page: 0,
            native_id: id.to_string(),
            subtype: "Highlight".to_string(),
            contents: contents.to_string(),
            color: "yellow".to_string(),
            rect: Some(rect),
            has_quad_points: true,
            fingerprint: annotation_fingerprint("Highlight", contents, "yellow", Some(rect)),
        }
    }

    fn page_text() -> Vec<PdfPageText> {
        vec![PdfPageText {
            width_pts: 600.0,
            height_pts: 800.0,
            lines: vec![PdfTextLine {
                text: "Bound to these words".to_string(),
                rect: [60.0, 680.0, 300.0, 700.0],
            }],
            ..PdfPageText::default()
        }]
    }

    #[test]
    fn annotation_projection_requires_matching_page_identity_content_and_geometry() {
        {
            let annotation = native("native-2", "");
            let original = PdfAnnotationSnapshot {
                document_fingerprint: "doc-a".to_string(),
                pages: vec![page("page-a")],
                annotations: vec![annotation.clone()],
            };
            let mut file = AnnotationsFile::default();
            reconcile(&mut file, &original, &page_text());

            let changed = PdfAnnotationSnapshot {
                document_fingerprint: "doc-b".to_string(),
                pages: vec![page("page-b")],
                annotations: vec![annotation],
            };
            let result = reconcile(&mut file, &changed, &page_text());
            let state = result.states[&file.annotations[0].id];
            assert_eq!(state, PdfAnnotationState::Orphaned);
            assert!(!state.is_projectable());
            assert_eq!(
                file.annotations.len(),
                1,
                "the native peer is not re-imported"
            );
        }
        {
            let annotation = native("native-3", "");
            let original = PdfAnnotationSnapshot {
                document_fingerprint: "doc-a".to_string(),
                pages: vec![page("page-a")],
                annotations: vec![annotation.clone()],
            };
            let mut file = AnnotationsFile::default();
            reconcile(&mut file, &original, &page_text());

            let changed = PdfAnnotationSnapshot {
                document_fingerprint: "doc-b".to_string(),
                pages: vec![page("page-a")],
                annotations: vec![annotation],
            };
            let result = reconcile(&mut file, &changed, &page_text());
            let state = result.states[&file.annotations[0].id];
            assert_eq!(state, PdfAnnotationState::VerifiedAfterDocumentChange);
            assert!(state.is_projectable());
        }
        {
            let annotation = native("native-4", "first comment");
            let snapshot = PdfAnnotationSnapshot {
                document_fingerprint: "doc-a".to_string(),
                pages: vec![page("page-a")],
                annotations: vec![annotation.clone()],
            };
            let mut file = AnnotationsFile::default();
            reconcile(&mut file, &snapshot, &page_text());

            let mut changed_native = annotation;
            changed_native.contents = "edited elsewhere".to_string();
            changed_native.fingerprint = annotation_fingerprint(
                "Highlight",
                &changed_native.contents,
                "yellow",
                changed_native.rect,
            );
            let changed = PdfAnnotationSnapshot {
                annotations: vec![changed_native],
                ..snapshot
            };
            let result = reconcile(&mut file, &changed, &page_text());
            let state = result.states[&file.annotations[0].id];
            assert_eq!(state, PdfAnnotationState::Conflict);
            assert!(!state.is_projectable());
        }
    }
}
