use fs::Fs;
use fuzzy::{StringMatch, StringMatchCandidate, match_strings};
use gpui::{
    App, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, ParentElement,
    Render, Styled, WeakEntity, Window, actions,
};
use language::language_settings::{AllLanguageSettings, EditPredictionProvider, FeaturesContent};
use picker::{Picker, PickerDelegate};
use settings::update_settings_file;
use std::{str::FromStr, sync::Arc};
use strum::IntoEnumIterator;
use ui::{HighlightedLabel, ListItem, ListItemSpacing, prelude::*};
use util::ResultExt;
use workspace::{ModalView, Workspace};

actions!(prediction_provider_selector, [Toggle]);

pub fn init(cx: &mut App) {
    cx.observe_new(PredictionProviderSelector::register)
        .detach();
}

pub struct PredictionProviderSelector {
    picker: Entity<Picker<PredictionProviderSelectorDelegate>>,
}

impl PredictionProviderSelector {
    fn register(
        workspace: &mut Workspace,
        _window: Option<&mut Window>,
        _: &mut Context<Workspace>,
    ) {
        workspace.register_action(move |workspace, _: &Toggle, window, cx| {
            Self::toggle(workspace, window, workspace.app_state().fs.clone(), cx);
        });
    }

    fn toggle(
        workspace: &mut Workspace,
        window: &mut Window,
        fs: Arc<dyn Fs>,
        cx: &mut Context<Workspace>,
    ) -> Option<()> {
        workspace.toggle_modal(window, cx, move |window, cx| {
            PredictionProviderSelector::new(window, fs, cx)
        });
        Some(())
    }

    fn new(window: &mut Window, fs: Arc<dyn Fs>, cx: &mut Context<Self>) -> Self {
        let delegate = PredictionProviderSelectorDelegate::new(cx.entity().downgrade(), fs);

        let picker = cx.new(|cx| Picker::uniform_list(delegate, window, cx));
        Self { picker }
    }
}

impl Render for PredictionProviderSelector {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        v_flex().w(rems(34.)).child(self.picker.clone())
    }
}

impl Focusable for PredictionProviderSelector {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.picker.focus_handle(cx)
    }
}

impl EventEmitter<DismissEvent> for PredictionProviderSelector {}
impl ModalView for PredictionProviderSelector {}

pub struct PredictionProviderSelectorDelegate {
    language_selector: WeakEntity<PredictionProviderSelector>,
    candidates: Vec<StringMatchCandidate>,
    matches: Vec<StringMatch>,
    selected_index: usize,
    fs: Arc<dyn Fs>,
}

impl PredictionProviderSelectorDelegate {
    fn new(language_selector: WeakEntity<PredictionProviderSelector>, fs: Arc<dyn Fs>) -> Self {
        let candidates = EditPredictionProvider::iter()
            .enumerate()
            .filter_map(|(i, provider)| {
                if provider == EditPredictionProvider::None {
                    return None;
                }
                Some(StringMatchCandidate::new(i, &provider.to_string()))
            })
            .collect::<Vec<_>>();

        Self {
            language_selector,
            candidates,
            matches: vec![],
            selected_index: 0,
            fs,
        }
    }

    fn icon_for_match(&self, selected: &Option<&StringMatch>) -> Option<Icon> {
        if let Some(selected) = selected {
            if let Ok(provider) = EditPredictionProvider::from_str(&selected.string) {
                return match provider {
                    EditPredictionProvider::Zed => Some(Icon::new(IconName::ZedPredict)),
                    EditPredictionProvider::Copilot => Some(Icon::new(IconName::Copilot)),
                    EditPredictionProvider::Supermaven => Some(Icon::new(IconName::Supermaven)),
                    EditPredictionProvider::None => None,
                };
            }
        }
        None
    }
}

impl PickerDelegate for PredictionProviderSelectorDelegate {
    type ListItem = ListItem;

    fn placeholder_text(&self, _window: &mut Window, _cx: &mut App) -> Arc<str> {
        "Select a prediction provider…".into()
    }

    fn match_count(&self) -> usize {
        self.matches.len()
    }

    fn confirm(&mut self, _: bool, window: &mut Window, cx: &mut Context<Picker<Self>>) {
        if let Some(mat) = self.matches.get(self.selected_index) {
            let selected = mat.string.clone();
            let chosen = EditPredictionProvider::from_str(&selected);
            if let Ok(provider) = chosen {
                update_settings_file::<AllLanguageSettings>(
                    self.fs.clone(),
                    cx,
                    move |settings, _| {
                        if let Some(features) = settings.features.as_mut() {
                            features.edit_prediction_provider = Some(provider);
                        } else {
                            settings.features = Some(FeaturesContent {
                                edit_prediction_provider: Some(provider),
                                ..Default::default()
                            });
                        }
                    },
                );
            }
        }
        self.dismissed(window, cx);
    }

    fn dismissed(&mut self, _: &mut Window, cx: &mut Context<Picker<Self>>) {
        self.language_selector
            .update(cx, |_, cx| cx.emit(DismissEvent))
            .log_err();
    }

    fn selected_index(&self) -> usize {
        self.selected_index
    }

    fn set_selected_index(
        &mut self,
        ix: usize,
        _window: &mut Window,
        _: &mut Context<Picker<Self>>,
    ) {
        self.selected_index = ix;
    }

    fn update_matches(
        &mut self,
        query: String,
        window: &mut Window,
        cx: &mut Context<Picker<Self>>,
    ) -> gpui::Task<()> {
        let background = cx.background_executor().clone();
        let candidates = self.candidates.clone();
        cx.spawn_in(window, async move |this, cx| {
            let matches = if query.is_empty() {
                candidates
                    .into_iter()
                    .enumerate()
                    .map(|(index, candidate)| StringMatch {
                        candidate_id: index,
                        string: candidate.string,
                        positions: Vec::new(),
                        score: 0.0,
                    })
                    .collect()
            } else {
                match_strings(
                    candidates.as_slice(),
                    &query,
                    false,
                    100,
                    &Default::default(),
                    background,
                )
                .await
            };

            this.update(cx, |this, cx| {
                let delegate = &mut this.delegate;
                delegate.matches = matches;
                delegate.selected_index = delegate
                    .selected_index
                    .min(delegate.matches.len().saturating_sub(1));
                cx.notify();
            })
            .log_err();
        })
    }

    fn render_match(
        &self,
        ix: usize,
        selected: bool,
        _: &mut Window,
        _cx: &mut Context<Picker<Self>>,
    ) -> Option<Self::ListItem> {
        let mat = &self.matches.get(ix);
        let icon = self.icon_for_match(mat);
        if let Some(mat) = mat {
            Some(
                ListItem::new(ix)
                    .inset(true)
                    .spacing(ListItemSpacing::Sparse)
                    .toggle_state(selected)
                    .start_slot::<Icon>(icon)
                    .child(HighlightedLabel::new(
                        mat.string.clone(),
                        mat.positions.clone(),
                    )),
            )
        } else {
            None
        }
    }
}
