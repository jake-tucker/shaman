//! Model option resolution and atomic option updates.

use crate::tui_rendering::support::{
    deliver_settings, left_click_at, model_descriptor, point_at, rendered_application_buffer,
    rendered_application_rows, rendered_application_rows_at, selected_session_snapshot,
    text_position, type_terminal_text, workspace_dir,
};
use crossterm::event::{Event as InputEvent, KeyCode, KeyEvent, KeyModifiers};
use suru::{
    protocol::{
        AgentSelection, ModelAvailability, ModelCatalog, ModelId, ModelOptionChoice,
        ModelOptionChoiceId, ModelOptionDescriptor, ModelOptionId, ModelOptionKind,
        ModelOptionRole, ModelOptionSelection, ModelOptionValue, ProviderCatalogStatus, ProviderId,
        ProviderModelCatalog, SessionId,
    },
    tui::{Application, ApplicationEvent, ApplicationTransition, CommandId},
};

#[test]
fn options_command_resolves_provider_default_and_explains_unavailable_configuration() {
    let mut application = Application::default();
    let ApplicationTransition::ListModels(request) = application
        .handle_event(ApplicationEvent::Command(CommandId::InvokeSemantic(
            suru::tui::SemanticCommandId::ModelOptions,
        )))
        .expect("invoke options without a concrete Model")
    else {
        panic!("options should resolve through the Model catalog");
    };
    let mut configurable = model_descriptor(
        "codex",
        "default-configurable",
        "Default Configurable",
        true,
        ModelAvailability::Available,
    );
    configurable.options.push(ModelOptionDescriptor {
        id: ModelOptionId::new("reasoning_effort"),
        label: "Reasoning".to_owned(),
        description: None,
        role: ModelOptionRole::ReasoningEffort,
        kind: ModelOptionKind::Select {
            choices: vec![
                ModelOptionChoice {
                    id: ModelOptionChoiceId::new("medium"),
                    label: "Medium".to_owned(),
                    description: None,
                    availability: ModelAvailability::Available,
                },
                ModelOptionChoice {
                    id: ModelOptionChoiceId::new("high"),
                    label: "High".to_owned(),
                    description: None,
                    availability: ModelAvailability::Available,
                },
            ],
            default: ModelOptionChoiceId::new("medium"),
        },
    });
    application
        .handle_event(ApplicationEvent::ModelsListed {
            request,
            catalog: ModelCatalog {
                providers: vec![ProviderModelCatalog {
                    provider: ProviderId::new("codex"),
                    display_name: "codex".to_owned(),
                    models: vec![configurable],
                    status: ProviderCatalogStatus::Fresh,
                }],
            },
        })
        .expect("resolve the default configurable Model");
    let resolved = rendered_application_rows(&application).join("\n");
    assert!(resolved.contains("Model Options"));
    assert!(resolved.contains("Default Configurable"));
    assert!(resolved.contains("Reasoning · Medium"));

    application
        .handle_terminal_event(InputEvent::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )))
        .expect("open default Reasoning choices");
    application
        .handle_terminal_event(InputEvent::Key(KeyEvent::new(
            KeyCode::Down,
            KeyModifiers::NONE,
        )))
        .expect("focus non-default reasoning");
    application
        .handle_terminal_event(InputEvent::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )))
        .expect("stage non-default reasoning");
    assert!(
        rendered_application_rows(&application)
            .join("\n")
            .contains("Reasoning · High")
    );
    application
        .handle_terminal_event(InputEvent::Key(KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::NONE,
        )))
        .expect("cancel default options without mutation");
    type_terminal_text(&mut application, "No implicit mutation");
    let ApplicationTransition::CreateSession(request) = application
        .handle_event(ApplicationEvent::Command(CommandId::SubmitSteer))
        .expect("submit after cancelling options")
    else {
        panic!("landing Prompt should create a Session");
    };
    assert_eq!(request.agent_selection, None);

    let mut unavailable = Application::default();
    let ApplicationTransition::ListModels(request) = unavailable
        .handle_event(ApplicationEvent::Command(CommandId::InvokeSemantic(
            suru::tui::SemanticCommandId::ModelOptions,
        )))
        .expect("invoke unavailable options")
    else {
        panic!("options should resolve through the Model catalog");
    };
    unavailable
        .handle_event(ApplicationEvent::ModelsRefreshed {
            request,
            catalog: ModelCatalog {
                providers: vec![ProviderModelCatalog {
                    provider: ProviderId::new("codex"),
                    display_name: "codex".to_owned(),
                    models: vec![model_descriptor(
                        "codex",
                        "plain",
                        "Plain Model",
                        true,
                        ModelAvailability::Available,
                    )],
                    status: ProviderCatalogStatus::Fresh,
                }],
            },
        })
        .expect("resolve a Model without options");
    let status = rendered_application_rows(&unavailable).join("\n");
    assert!(!status.contains("Model Options"));
    assert!(status.contains("Plain Model has no configurable options"));
    assert!(status.contains("/models"));

    assert!(matches!(
        unavailable
            .handle_event(ApplicationEvent::Command(CommandId::InvokeSemantic(
                suru::tui::SemanticCommandId::ModelList,
            )))
            .expect("choose the same plain Model through /models"),
        ApplicationTransition::ListModels(_)
    ));
    assert!(matches!(
        unavailable
            .handle_terminal_event(InputEvent::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE,
            )))
            .expect("apply a Model without descriptors immediately"),
        ApplicationTransition::ConfirmLandingAgentSelection(_)
    ));
    assert!(
        !rendered_application_rows(&unavailable)
            .join("\n")
            .contains("Model Options")
    );
    type_terminal_text(&mut unavailable, "Use plain Model");
    let ApplicationTransition::CreateSession(request) = unavailable
        .handle_event(ApplicationEvent::Command(CommandId::SubmitSteer))
        .expect("submit plain Model selection")
    else {
        panic!("landing Prompt should create a Session");
    };
    assert_eq!(
        request.agent_selection,
        Some(AgentSelection {
            provider: ProviderId::new("codex"),
            model: ModelId::new("plain"),
            options: Vec::new(),
        })
    );

    let mut missing = Application::default();
    let ApplicationTransition::ListModels(request) = missing
        .handle_event(ApplicationEvent::Command(CommandId::InvokeSemantic(
            suru::tui::SemanticCommandId::ModelOptions,
        )))
        .expect("invoke options without any available Model")
    else {
        panic!("options should request the catalog");
    };
    missing
        .handle_event(ApplicationEvent::ModelsRefreshed {
            request,
            catalog: ModelCatalog {
                providers: Vec::new(),
            },
        })
        .expect("finish empty catalog resolution");
    let status = rendered_application_rows(&missing).join("\n");
    assert!(status.contains("No concrete Model is available"));
    assert!(status.contains("/models"));

    let mut ambiguous = Application::default();
    let ApplicationTransition::ListModels(request) = ambiguous
        .handle_event(ApplicationEvent::Command(CommandId::InvokeSemantic(
            suru::tui::SemanticCommandId::ModelOptions,
        )))
        .expect("invoke options without a selected Provider")
    else {
        panic!("options should request the catalog");
    };
    let provider_default = |provider: &str| {
        let mut model = model_descriptor(
            provider,
            "default",
            &format!("{provider} default"),
            true,
            ModelAvailability::Available,
        );
        model.options.push(ModelOptionDescriptor {
            id: ModelOptionId::new("fast"),
            label: "Fast".to_owned(),
            description: None,
            role: ModelOptionRole::Speed,
            kind: ModelOptionKind::Toggle { default: false },
        });
        ProviderModelCatalog {
            provider: ProviderId::new(provider),
            display_name: provider.to_owned(),
            models: vec![model],
            status: ProviderCatalogStatus::Fresh,
        }
    };
    ambiguous
        .handle_event(ApplicationEvent::ModelsRefreshed {
            request,
            catalog: ModelCatalog {
                providers: vec![provider_default("codex"), provider_default("copilot")],
            },
        })
        .expect("finish ambiguous Provider resolution");
    let status = rendered_application_rows(&ambiguous).join("\n");
    assert!(!status.contains("Model Options"));
    assert!(status.contains("No concrete Model is available"));
    assert!(status.contains("/models"));
}

#[test]
fn stale_catalog_failure_does_not_end_newer_options_resolution() {
    let mut application = Application::default();
    let ApplicationTransition::ListModels(stale_request) = application
        .handle_event(ApplicationEvent::Command(CommandId::InvokeSemantic(
            suru::tui::SemanticCommandId::ModelOptions,
        )))
        .expect("begin first options resolution")
    else {
        panic!("options should request the catalog");
    };
    let ApplicationTransition::ListModels(current_request) = application
        .handle_event(ApplicationEvent::Command(CommandId::InvokeSemantic(
            suru::tui::SemanticCommandId::ModelOptions,
        )))
        .expect("replace options resolution")
    else {
        panic!("options should replace its catalog request");
    };
    application
        .handle_event(ApplicationEvent::ModelListingFailed {
            request: stale_request,
            error: "stale failure".to_owned(),
        })
        .expect("ignore stale failure");

    let mut model = model_descriptor(
        "codex",
        "current-default",
        "Current Default",
        true,
        ModelAvailability::Available,
    );
    model.options.push(ModelOptionDescriptor {
        id: ModelOptionId::new("fast"),
        label: "Fast".to_owned(),
        description: None,
        role: ModelOptionRole::Speed,
        kind: ModelOptionKind::Toggle { default: false },
    });
    application
        .handle_event(ApplicationEvent::ModelsListed {
            request: current_request,
            catalog: ModelCatalog {
                providers: vec![ProviderModelCatalog {
                    provider: ProviderId::new("codex"),
                    display_name: "codex".to_owned(),
                    models: vec![model],
                    status: ProviderCatalogStatus::Fresh,
                }],
            },
        })
        .expect("resolve the newest options request");
    let screen = rendered_application_rows(&application).join("\n");
    assert!(screen.contains("Model Options"));
    assert!(screen.contains("Current Default"));
    assert!(!screen.contains("stale failure"));
}

#[test]
fn refreshed_descriptors_replace_a_cached_no_options_status() {
    let mut application = Application::default();
    let ApplicationTransition::ListModels(cached_request) = application
        .handle_event(ApplicationEvent::Command(CommandId::InvokeSemantic(
            suru::tui::SemanticCommandId::ModelList,
        )))
        .expect("open Model picker to seed cache")
    else {
        panic!("Model picker should request the catalog");
    };
    let plain = model_descriptor(
        "codex",
        "changing",
        "Changing Model",
        true,
        ModelAvailability::Available,
    );
    application
        .handle_event(ApplicationEvent::ModelsListed {
            request: cached_request,
            catalog: ModelCatalog {
                providers: vec![ProviderModelCatalog {
                    provider: ProviderId::new("codex"),
                    display_name: "codex".to_owned(),
                    models: vec![plain.clone()],
                    status: ProviderCatalogStatus::Refreshing,
                }],
            },
        })
        .expect("seed cached Model without descriptors");
    application
        .handle_terminal_event(InputEvent::Key(KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::NONE,
        )))
        .expect("close Model picker");

    let ApplicationTransition::ListModels(options_request) = application
        .handle_event(ApplicationEvent::Command(CommandId::InvokeSemantic(
            suru::tui::SemanticCommandId::ModelOptions,
        )))
        .expect("resolve options from stale cache")
    else {
        panic!("options should refresh the catalog");
    };
    assert!(
        rendered_application_rows(&application)
            .join("\n")
            .contains("Changing Model has no configurable options")
    );

    let mut configurable = plain;
    configurable.options.push(ModelOptionDescriptor {
        id: ModelOptionId::new("fast"),
        label: "Fast".to_owned(),
        description: Some("Prefer low latency".to_owned()),
        role: ModelOptionRole::Speed,
        kind: ModelOptionKind::Toggle { default: false },
    });
    application
        .handle_event(ApplicationEvent::ModelsRefreshed {
            request: options_request,
            catalog: ModelCatalog {
                providers: vec![ProviderModelCatalog {
                    provider: ProviderId::new("codex"),
                    display_name: "codex".to_owned(),
                    models: vec![configurable],
                    status: ProviderCatalogStatus::Fresh,
                }],
            },
        })
        .expect("replace stale no-options descriptor");
    let screen = rendered_application_rows(&application).join("\n");
    assert!(screen.contains("Model Options"));
    assert!(screen.contains("Fast · Off"));
    assert!(!screen.contains("has no configurable options"));
}

#[test]
fn session_options_preserve_other_dimensions_and_roll_back_one_atomic_update() {
    let workspace = workspace_dir();
    let session_id = SessionId::new();
    let authoritative = AgentSelection {
        provider: ProviderId::new("codex"),
        model: ModelId::new("gpt-configurable"),
        options: vec![
            suru::protocol::ModelOptionSelection {
                id: ModelOptionId::new("reasoning_effort"),
                value: ModelOptionValue::Select {
                    choice: ModelOptionChoiceId::new("low"),
                },
            },
            suru::protocol::ModelOptionSelection {
                id: ModelOptionId::new("fast"),
                value: ModelOptionValue::Toggle { enabled: false },
            },
        ],
    };
    let mut application = Application::new(workspace.path(), Default::default());
    application
        .handle_event(ApplicationEvent::SessionAttached(
            selected_session_snapshot(session_id, workspace.path(), authoritative.clone()),
        ))
        .expect("attach selected Session");

    let ApplicationTransition::ListModels(catalog_request) = application
        .handle_event(ApplicationEvent::Command(CommandId::InvokeSemantic(
            suru::tui::SemanticCommandId::ModelOptions,
        )))
        .expect("open authoritative Model Options")
    else {
        panic!("options should request the Model catalog");
    };
    let mut model = model_descriptor(
        "codex",
        "gpt-configurable",
        "Configurable GPT",
        true,
        ModelAvailability::Available,
    );
    model.options = vec![
        ModelOptionDescriptor {
            id: ModelOptionId::new("reasoning_effort"),
            label: "Reasoning".to_owned(),
            description: None,
            role: ModelOptionRole::ReasoningEffort,
            kind: ModelOptionKind::Select {
                choices: vec![
                    ModelOptionChoice {
                        id: ModelOptionChoiceId::new("low"),
                        label: "Low".to_owned(),
                        description: None,
                        availability: ModelAvailability::Available,
                    },
                    ModelOptionChoice {
                        id: ModelOptionChoiceId::new("high"),
                        label: "High".to_owned(),
                        description: None,
                        availability: ModelAvailability::Available,
                    },
                ],
                default: ModelOptionChoiceId::new("high"),
            },
        },
        ModelOptionDescriptor {
            id: ModelOptionId::new("fast"),
            label: "Fast".to_owned(),
            description: None,
            role: ModelOptionRole::Speed,
            kind: ModelOptionKind::Toggle { default: true },
        },
    ];
    application
        .handle_event(ApplicationEvent::ModelsListed {
            request: catalog_request,
            catalog: ModelCatalog {
                providers: vec![ProviderModelCatalog {
                    provider: ProviderId::new("codex"),
                    display_name: "codex".to_owned(),
                    models: vec![model],
                    status: ProviderCatalogStatus::Fresh,
                }],
            },
        })
        .expect("load current Model Options");
    let reopened = rendered_application_rows(&application).join("\n");
    assert!(reopened.contains("Reasoning · Low"));
    assert!(reopened.contains("Fast · Off"));

    application
        .handle_terminal_event(InputEvent::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )))
        .expect("open Reasoning choices");
    application
        .handle_terminal_event(InputEvent::Key(KeyEvent::new(
            KeyCode::Down,
            KeyModifiers::NONE,
        )))
        .expect("focus High");
    application
        .handle_terminal_event(InputEvent::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )))
        .expect("stage High");

    let ApplicationTransition::UpdateAgentSelection {
        session: updated_session,
        request,
    } = application
        .handle_terminal_event(InputEvent::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::CONTROL,
        )))
        .expect("apply one complete Agent Selection")
    else {
        panic!("Session options should produce one authoritative update");
    };
    assert_eq!(updated_session.origin, suru::protocol::Outlook::Local);
    assert_eq!(updated_session.session_id, session_id);
    assert_eq!(
        request.selection.options,
        vec![
            suru::protocol::ModelOptionSelection {
                id: ModelOptionId::new("reasoning_effort"),
                value: ModelOptionValue::Select {
                    choice: ModelOptionChoiceId::new("high"),
                },
            },
            suru::protocol::ModelOptionSelection {
                id: ModelOptionId::new("fast"),
                value: ModelOptionValue::Toggle { enabled: false },
            },
        ]
    );
    let optimistic = rendered_application_rows_at(&application, 100, 16).join("\n");
    assert!(optimistic.contains("codex · Configurable GPT · High · Off"));
    type_terminal_text(&mut application, "must wait");
    assert_eq!(
        application
            .handle_event(ApplicationEvent::Command(CommandId::SubmitSteer))
            .expect("block Prompt while options settle"),
        ApplicationTransition::Continue
    );
    assert_eq!(
        application
            .handle_event(ApplicationEvent::Command(CommandId::InvokeSemantic(
                suru::tui::SemanticCommandId::SessionList,
            )))
            .expect("block Session navigation while options settle"),
        ApplicationTransition::Continue
    );

    application
        .handle_event(ApplicationEvent::AgentSelectionUpdateFailed {
            operation_id: request.operation_id,
            error: "Options rejected".to_owned(),
        })
        .expect("reject complete options update");
    assert!(
        rendered_application_rows_at(&application, 100, 16)
            .join("\n")
            .contains("Options rejected")
    );
    assert!(matches!(
        application
            .handle_event(ApplicationEvent::Command(CommandId::InvokeSemantic(
                suru::tui::SemanticCommandId::ModelOptions,
            )))
            .expect("reopen rolled-back options"),
        ApplicationTransition::ListModels(_)
    ));
    let rolled_back = rendered_application_rows(&application).join("\n");
    assert!(rolled_back.contains("Reasoning · Low"));
    assert!(rolled_back.contains("Fast · Off"));
}

#[test]
fn refreshed_options_keep_invalidated_choice_visible_and_disable_apply() {
    let workspace = workspace_dir();
    let session_id = SessionId::new();
    let current = AgentSelection {
        provider: ProviderId::new("codex"),
        model: ModelId::new("changing"),
        options: vec![
            suru::protocol::ModelOptionSelection {
                id: ModelOptionId::new("reasoning_effort"),
                value: ModelOptionValue::Select {
                    choice: ModelOptionChoiceId::new("high"),
                },
            },
            suru::protocol::ModelOptionSelection {
                id: ModelOptionId::new("fast"),
                value: ModelOptionValue::Toggle { enabled: false },
            },
        ],
    };
    let mut application = Application::new(workspace.path(), Default::default());
    application
        .handle_event(ApplicationEvent::SessionAttached(
            selected_session_snapshot(session_id, workspace.path(), current),
        ))
        .expect("attach selected Session");
    let ApplicationTransition::ListModels(request) = application
        .handle_event(ApplicationEvent::Command(CommandId::InvokeSemantic(
            suru::tui::SemanticCommandId::ModelOptions,
        )))
        .expect("open options")
    else {
        panic!("options should request the catalog");
    };
    let option = |choices: Vec<ModelOptionChoice>| ModelOptionDescriptor {
        id: ModelOptionId::new("reasoning_effort"),
        label: "Reasoning".to_owned(),
        description: None,
        role: ModelOptionRole::ReasoningEffort,
        kind: ModelOptionKind::Select {
            choices,
            default: ModelOptionChoiceId::new("low"),
        },
    };
    let low = || ModelOptionChoice {
        id: ModelOptionChoiceId::new("low"),
        label: "Low".to_owned(),
        description: None,
        availability: ModelAvailability::Available,
    };
    let high = ModelOptionChoice {
        id: ModelOptionChoiceId::new("high"),
        label: "High".to_owned(),
        description: None,
        availability: ModelAvailability::Available,
    };
    let mut initial = model_descriptor(
        "codex",
        "changing",
        "Changing Model",
        true,
        ModelAvailability::Available,
    );
    initial.options = vec![
        option(vec![low(), high]),
        ModelOptionDescriptor {
            id: ModelOptionId::new("fast"),
            label: "Fast".to_owned(),
            description: None,
            role: ModelOptionRole::Speed,
            kind: ModelOptionKind::Toggle { default: false },
        },
    ];
    application
        .handle_event(ApplicationEvent::ModelsListed {
            request: request.clone(),
            catalog: ModelCatalog {
                providers: vec![ProviderModelCatalog {
                    provider: ProviderId::new("codex"),
                    display_name: "codex".to_owned(),
                    models: vec![initial.clone()],
                    status: ProviderCatalogStatus::Refreshing,
                }],
            },
        })
        .expect("open cached options");

    application
        .handle_terminal_event(InputEvent::Key(KeyEvent::new(
            KeyCode::Down,
            KeyModifiers::NONE,
        )))
        .expect("focus Fast");
    application
        .handle_terminal_event(InputEvent::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )))
        .expect("open Fast choices");
    application
        .handle_terminal_event(InputEvent::Key(KeyEvent::new(
            KeyCode::Down,
            KeyModifiers::NONE,
        )))
        .expect("focus On");
    application
        .handle_terminal_event(InputEvent::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )))
        .expect("stage Fast On");

    let mut refreshed = initial;
    refreshed.options[0] = option(vec![low()]);
    application
        .handle_event(ApplicationEvent::ModelsRefreshed {
            request,
            catalog: ModelCatalog {
                providers: vec![ProviderModelCatalog {
                    provider: ProviderId::new("codex"),
                    display_name: "codex".to_owned(),
                    models: vec![refreshed],
                    status: ProviderCatalogStatus::Fresh,
                }],
            },
        })
        .expect("invalidate High reasoning");
    let invalid = rendered_application_rows(&application).join("\n");
    assert!(invalid.contains("Reasoning · high (unavailable) [unavailable]"));
    assert!(invalid.contains("Fast · On"));
    assert!(invalid.contains("Apply unavailable"));
    application
        .handle_terminal_event(InputEvent::Key(KeyEvent::new(
            KeyCode::Down,
            KeyModifiers::NONE,
        )))
        .expect("focus disabled Confirm");
    assert!(
        rendered_application_rows(&application)
            .join("\n")
            .contains("› Confirm [unavailable]")
    );
    assert_eq!(
        application
            .handle_terminal_event(InputEvent::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE
            )))
            .expect("refuse invalid confirmation"),
        ApplicationTransition::Continue
    );
    application
        .handle_terminal_event(InputEvent::Key(KeyEvent::new(
            KeyCode::Up,
            KeyModifiers::NONE,
        )))
        .expect("return to Fast");
    assert_eq!(
        application
            .handle_terminal_event(InputEvent::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::CONTROL,
            )))
            .expect("refuse invalid complete selection"),
        ApplicationTransition::Continue
    );
    assert!(
        rendered_application_rows(&application)
            .join("\n")
            .contains("Model Options")
    );

    application
        .handle_terminal_event(InputEvent::Key(KeyEvent::new(
            KeyCode::Up,
            KeyModifiers::NONE,
        )))
        .expect("focus invalid Reasoning");
    application
        .handle_terminal_event(InputEvent::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )))
        .expect("open choices including invalid current value");
    let choices = rendered_application_rows(&application).join("\n");
    assert!(choices.contains("high [current] [unavailable]"));
    application
        .handle_terminal_event(InputEvent::Key(KeyEvent::new(
            KeyCode::Down,
            KeyModifiers::NONE,
        )))
        .expect("explicitly focus Low");
    application
        .handle_terminal_event(InputEvent::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )))
        .expect("replace invalid choice");
    let ApplicationTransition::UpdateAgentSelection { request, .. } = application
        .handle_terminal_event(InputEvent::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::CONTROL,
        )))
        .expect("apply repaired complete selection")
    else {
        panic!("valid repaired options should apply");
    };
    assert_eq!(
        request.selection.options,
        vec![
            suru::protocol::ModelOptionSelection {
                id: ModelOptionId::new("reasoning_effort"),
                value: ModelOptionValue::Select {
                    choice: ModelOptionChoiceId::new("low"),
                },
            },
            suru::protocol::ModelOptionSelection {
                id: ModelOptionId::new("fast"),
                value: ModelOptionValue::Toggle { enabled: true },
            },
        ]
    );
}

/// A Model with the two dimensions Copilot advertises: reasoning effort, and a context tier that
/// trades context window against cost.
fn tiered_model() -> suru::protocol::ModelDescriptor {
    let mut model = model_descriptor(
        "copilot",
        "tiered",
        "Tiered Fixture",
        true,
        ModelAvailability::Available,
    );
    model.options = vec![
        ModelOptionDescriptor {
            id: ModelOptionId::new("reasoning_effort"),
            label: "Reasoning effort".to_owned(),
            description: None,
            role: ModelOptionRole::ReasoningEffort,
            kind: ModelOptionKind::Select {
                choices: vec![ModelOptionChoice {
                    id: ModelOptionChoiceId::new("high"),
                    label: "High".to_owned(),
                    description: None,
                    availability: ModelAvailability::Available,
                }],
                default: ModelOptionChoiceId::new("high"),
            },
        },
        ModelOptionDescriptor {
            id: ModelOptionId::new("context_tier"),
            label: "Context".to_owned(),
            description: None,
            role: ModelOptionRole::Context,
            kind: ModelOptionKind::Select {
                choices: vec![
                    ModelOptionChoice {
                        id: ModelOptionChoiceId::new("default"),
                        label: "Default".to_owned(),
                        description: None,
                        availability: ModelAvailability::Available,
                    },
                    ModelOptionChoice {
                        id: ModelOptionChoiceId::new("long_context"),
                        label: "Long context".to_owned(),
                        description: None,
                        availability: ModelAvailability::Available,
                    },
                ],
                default: ModelOptionChoiceId::new("default"),
            },
        },
    ];
    model
}

/// Opens the landing Model picker on `tiered_model` and selects it, leaving the Model Options panel
/// open on its Provider defaults.
fn landing_on_the_tiered_model() -> Application {
    let mut application = Application::default();
    let ApplicationTransition::ListModels(request) = application
        .handle_event(ApplicationEvent::Command(CommandId::InvokeSemantic(
            suru::tui::SemanticCommandId::ModelList,
        )))
        .expect("open the landing Model picker")
    else {
        panic!("the Model picker should request the catalog");
    };
    application
        .handle_event(ApplicationEvent::ModelsListed {
            request,
            catalog: ModelCatalog {
                providers: vec![ProviderModelCatalog {
                    provider: ProviderId::new("copilot"),
                    display_name: "copilot".to_owned(),
                    models: vec![tiered_model()],
                    status: ProviderCatalogStatus::Fresh,
                }],
            },
        })
        .expect("load the landing Models");
    application
        .handle_terminal_event(InputEvent::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )))
        .expect("select the tiered Model");
    application
}

#[test]
fn context_tier_is_configurable_and_surfaces_once_it_leaves_its_default() {
    let mut application = landing_on_the_tiered_model();
    assert!(
        rendered_application_rows(&application)
            .join("\n")
            .contains("Context · Default"),
        "the context tier is one of the Model's configurable Options"
    );

    application
        .handle_terminal_event(InputEvent::Key(KeyEvent::new(
            KeyCode::Down,
            KeyModifiers::NONE,
        )))
        .expect("focus the context tier");
    application
        .handle_terminal_event(InputEvent::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )))
        .expect("open the context tier choices");
    let choices = rendered_application_rows(&application).join("\n");
    assert!(choices.contains("Context Choices"));
    assert!(choices.contains("Default [current]"));
    assert!(choices.contains("Long context"));

    application
        .handle_terminal_event(InputEvent::Key(KeyEvent::new(
            KeyCode::Down,
            KeyModifiers::NONE,
        )))
        .expect("focus the extended tier");
    application
        .handle_terminal_event(InputEvent::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )))
        .expect("stage the extended tier");
    assert!(
        rendered_application_rows(&application)
            .join("\n")
            .contains("Context · Long context")
    );

    application
        .handle_terminal_event(InputEvent::Key(KeyEvent::new(
            KeyCode::Down,
            KeyModifiers::NONE,
        )))
        .expect("focus Confirm after the last option");
    assert!(
        rendered_application_rows(&application)
            .join("\n")
            .contains("› Confirm")
    );
    let ApplicationTransition::ConfirmLandingAgentSelection(confirmed) = application
        .handle_terminal_event(InputEvent::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )))
        .expect("apply the staged context tier")
    else {
        panic!("landing options should confirm the complete Agent Selection");
    };
    assert!(confirmed.options.contains(&ModelOptionSelection {
        id: ModelOptionId::new("context_tier"),
        value: ModelOptionValue::Select {
            choice: ModelOptionChoiceId::new("long_context"),
        },
    }));
    assert!(
        rendered_application_rows_at(&application, 140, 16)
            .join("\n")
            .contains("copilot · Tiered Fixture · High · Long context"),
        "a context tier off its default is worth a place in the Agent Selection summary"
    );
}

#[test]
fn a_context_tier_left_at_its_default_remains_a_present_selection_summary_field() {
    let mut application = landing_on_the_tiered_model();
    let ApplicationTransition::ConfirmLandingAgentSelection(confirmed) = application
        .handle_terminal_event(InputEvent::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::CONTROL,
        )))
        .expect("apply the Provider defaults")
    else {
        panic!("landing options should confirm the complete Agent Selection");
    };
    assert!(confirmed.options.contains(&ModelOptionSelection {
        id: ModelOptionId::new("context_tier"),
        value: ModelOptionValue::Select {
            choice: ModelOptionChoiceId::new("default"),
        },
    }));
    // Wide enough that every present field can be observed rather than truncated away.
    let summary = rendered_application_rows_at(&application, 140, 16).join("\n");
    assert!(
        summary.contains("copilot · Tiered Fixture · High · Default"),
        "a selected default tier is still a present Context field: {summary}"
    );
}

#[test]
fn selection_summary_orders_unlabelled_provider_model_and_supported_options() {
    let mut application = Application::default();
    let ApplicationTransition::ListModels(request) = application
        .handle_event(ApplicationEvent::Command(CommandId::InvokeSemantic(
            suru::tui::SemanticCommandId::ModelList,
        )))
        .expect("open the landing Model picker")
    else {
        panic!("the Model picker should request the catalog");
    };
    let mut model = model_descriptor(
        "codex",
        "gpt-5.6-luna",
        "GPT-5.6 Luna",
        true,
        ModelAvailability::Available,
    );
    model.options = vec![
        ModelOptionDescriptor {
            id: ModelOptionId::new("fast"),
            label: "Speed".to_owned(),
            description: None,
            role: ModelOptionRole::Speed,
            kind: ModelOptionKind::Toggle { default: true },
        },
        ModelOptionDescriptor {
            id: ModelOptionId::new("verbosity"),
            label: "Verbosity".to_owned(),
            description: None,
            role: ModelOptionRole::Verbosity,
            kind: ModelOptionKind::Toggle { default: true },
        },
        ModelOptionDescriptor {
            id: ModelOptionId::new("context_tier"),
            label: "Context".to_owned(),
            description: None,
            role: ModelOptionRole::Context,
            kind: ModelOptionKind::Select {
                choices: vec![ModelOptionChoice {
                    id: ModelOptionChoiceId::new("long_context"),
                    label: "Long context".to_owned(),
                    description: None,
                    availability: ModelAvailability::Available,
                }],
                default: ModelOptionChoiceId::new("long_context"),
            },
        },
        ModelOptionDescriptor {
            id: ModelOptionId::new("reasoning_effort"),
            label: "Reasoning effort".to_owned(),
            description: None,
            role: ModelOptionRole::ReasoningEffort,
            kind: ModelOptionKind::Select {
                choices: vec![ModelOptionChoice {
                    id: ModelOptionChoiceId::new("high"),
                    label: "High".to_owned(),
                    description: None,
                    availability: ModelAvailability::Available,
                }],
                default: ModelOptionChoiceId::new("high"),
            },
        },
    ];
    application
        .handle_event(ApplicationEvent::ModelsListed {
            request,
            catalog: ModelCatalog {
                providers: vec![ProviderModelCatalog {
                    provider: ProviderId::new("codex"),
                    display_name: "Codex".to_owned(),
                    models: vec![model],
                    status: ProviderCatalogStatus::Fresh,
                }],
            },
        })
        .expect("load the landing Model");
    application
        .handle_terminal_event(InputEvent::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )))
        .expect("select the landing Model");
    assert!(matches!(
        application
            .handle_terminal_event(InputEvent::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::CONTROL,
            )))
            .expect("apply the complete Agent Selection"),
        ApplicationTransition::ConfirmLandingAgentSelection(_)
    ));

    let screen = rendered_application_rows_at(&application, 140, 16).join("\n");
    assert!(
        screen.contains("Codex · GPT-5.6 Luna · High · Long context · On"),
        "the footer follows Provider, Model, Effort, Context, Speed order: {screen}"
    );
    assert!(!screen.contains("Verbosity On"));
    assert!(!screen.contains("Provider Codex"));
    assert!(!screen.contains("Model GPT-5.6 Luna"));
    assert!(!screen.contains("Reasoning effort High"));
    assert!(!screen.contains("Context Long context"));
    assert!(!screen.contains("Speed On"));
}

#[test]
fn confirm_stays_pinned_and_focused_across_navigation_and_catalog_refresh() {
    let mut application = landing_on_the_tiered_model();
    let ApplicationTransition::ListModels(request) = application
        .handle_event(ApplicationEvent::Command(CommandId::InvokeSemantic(
            suru::tui::SemanticCommandId::ModelOptions,
        )))
        .expect("refresh options")
    else {
        panic!("options should refresh the catalog");
    };
    let key = |code| InputEvent::Key(KeyEvent::new(code, KeyModifiers::NONE));
    let screen = rendered_application_rows_at(&application, 80, 12);
    let confirm_line = screen
        .iter()
        .position(|row| row.contains("  Confirm"))
        .expect("pinned Confirm");
    assert!(screen.iter().any(|row| row.contains("› Reasoning effort")));
    application
        .handle_terminal_event(key(KeyCode::Up))
        .expect("wrap to Confirm");
    assert!(rendered_application_rows_at(&application, 80, 12)[confirm_line].contains("› Confirm"));
    application
        .handle_terminal_event(key(KeyCode::Down))
        .expect("wrap to first option");
    assert!(
        rendered_application_rows_at(&application, 80, 12)
            .join("\n")
            .contains("› Reasoning effort")
    );
    application
        .handle_terminal_event(key(KeyCode::Up))
        .expect("return to Confirm");

    let mut model = tiered_model();
    for index in 0..12 {
        model.options.push(ModelOptionDescriptor {
            id: ModelOptionId::new(format!("extra_{index}")),
            label: format!("Extra {index}"),
            description: None,
            role: ModelOptionRole::Speed,
            kind: ModelOptionKind::Toggle { default: false },
        });
    }
    let expected = model.default_agent_selection();
    application
        .handle_event(ApplicationEvent::ModelsRefreshed {
            request,
            catalog: ModelCatalog {
                providers: vec![ProviderModelCatalog {
                    provider: ProviderId::new("copilot"),
                    display_name: "copilot".to_owned(),
                    models: vec![model],
                    status: ProviderCatalogStatus::Fresh,
                }],
            },
        })
        .expect("refresh with more options while Confirm is focused");
    let refreshed = rendered_application_rows_at(&application, 80, 12);
    assert!(refreshed[confirm_line].contains("› Confirm"));
    assert!(refreshed.iter().any(|row| row.contains("Extra 11")));
    application
        .handle_terminal_event(key(KeyCode::Up))
        .expect("move to last option");
    let scrolled = rendered_application_rows_at(&application, 80, 12);
    assert!(scrolled[confirm_line].contains("  Confirm"));
    assert!(scrolled.iter().any(|row| row.contains("› Extra 11")));
    application
        .handle_terminal_event(key(KeyCode::Down))
        .expect("move to Confirm");
    assert_eq!(
        application
            .handle_terminal_event(key(KeyCode::Enter))
            .expect("confirm refreshed selection"),
        ApplicationTransition::ConfirmLandingAgentSelection(expected)
    );
}

#[test]
fn short_options_panel_keeps_focused_option_above_confirm() {
    let mut application = landing_on_the_tiered_model();
    for height in [6, 7, 8, 9] {
        let screen = rendered_application_rows_at(&application, 80, height).join("\n");
        assert!(
            screen.contains("› Reasoning effort"),
            "focused option at height {height}: {screen}"
        );
        assert!(
            screen.contains("  Confirm"),
            "Confirm at height {height}: {screen}"
        );
    }
    assert!(
        rendered_application_rows_at(&application, 80, 5)
            .join("\n")
            .contains("› Reasoning effort")
    );
    application
        .handle_terminal_event(InputEvent::Key(KeyEvent::new(
            KeyCode::Up,
            KeyModifiers::NONE,
        )))
        .expect("focus Confirm in a one-row panel");
    assert!(
        rendered_application_rows_at(&application, 80, 5)
            .join("\n")
            .contains("› Confirm")
    );
}

/// `auto` as Copilot offers it with the Setting on: a Routing Tier and nothing else, one tier
/// disabled with Copilot's reason.
fn routed_model() -> suru::protocol::ModelDescriptor {
    let mut model = model_descriptor(
        "copilot",
        "auto",
        "Auto",
        true,
        ModelAvailability::Available,
    );
    let tier = |id: &str, label: &str, description: &str, availability| ModelOptionChoice {
        id: ModelOptionChoiceId::new(id),
        label: label.to_owned(),
        description: Some(description.to_owned()),
        availability,
    };
    model.options = vec![ModelOptionDescriptor {
        id: ModelOptionId::new(suru::protocol::ROUTING_TIER_OPTION_ID),
        label: "Routing tier".to_owned(),
        description: None,
        role: ModelOptionRole::RoutingTier,
        kind: ModelOptionKind::Select {
            choices: vec![
                tier(
                    "balance",
                    "Standard",
                    "Versatile models.",
                    ModelAvailability::Available,
                ),
                tier(
                    "hydrafusion",
                    "High",
                    "Capable models.",
                    ModelAvailability::Available,
                ),
                tier(
                    "hydrafusion-max",
                    "Max",
                    "Not on your plan. The most capable models.",
                    ModelAvailability::Unavailable,
                ),
            ],
            default: ModelOptionChoiceId::new("balance"),
        },
    }];
    model
}

fn landing_on_the_routed_model() -> Application {
    landing_on_model_with_options(routed_model())
}

fn landing_on_model_with_options(model: suru::protocol::ModelDescriptor) -> Application {
    let mut application = Application::default();
    let ApplicationTransition::ListModels(request) = application
        .handle_event(ApplicationEvent::Command(CommandId::InvokeSemantic(
            suru::tui::SemanticCommandId::ModelList,
        )))
        .expect("open the landing Model picker")
    else {
        panic!("the Model picker should request the catalog");
    };
    application
        .handle_event(ApplicationEvent::ModelsListed {
            request,
            catalog: ModelCatalog {
                providers: vec![ProviderModelCatalog {
                    provider: model.provider.clone(),
                    display_name: match model.provider.as_str() {
                        "copilot" => "Copilot",
                        "codex" => "Codex",
                        "claude" => "Claude",
                        provider => provider,
                    }
                    .to_owned(),
                    models: vec![model],
                    status: ProviderCatalogStatus::Fresh,
                }],
            },
        })
        .expect("load the landing Models");
    application
        .handle_terminal_event(InputEvent::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )))
        .expect("select the routed Model");
    application
}

fn press(
    application: &mut Application,
    code: KeyCode,
    modifiers: KeyModifiers,
) -> ApplicationTransition {
    application
        .handle_terminal_event(InputEvent::Key(KeyEvent::new(code, modifiers)))
        .expect("handle a key")
}

/// Copilot's wording in Copilot's order, Copilot's default preselected, and a disabled tier shown
/// with Copilot's reason but not to be had.
#[test]
fn a_routing_tier_is_chosen_beside_the_model_and_a_disabled_one_says_why_and_cannot_be() {
    let mut application = landing_on_the_routed_model();
    let options = rendered_application_rows_at(&application, 140, 18).join("\n");
    assert!(
        options.contains("Routing tier") && options.contains("Standard"),
        "the default is preselected: {options}"
    );

    press(&mut application, KeyCode::Enter, KeyModifiers::NONE);
    let choices = rendered_application_rows_at(&application, 140, 18).join("\n");
    let standard = choices.find("Standard").expect("Standard is listed");
    let high = choices.find("High").expect("High is listed");
    let max = choices.find("Max").expect("Max is listed");
    assert!(standard < high && high < max, "Copilot's order: {choices}");
    assert!(
        choices.contains("Max [unavailable]") && !choices.contains("Not on your plan."),
        "{choices}"
    );

    press(&mut application, KeyCode::Down, KeyModifiers::NONE);
    press(&mut application, KeyCode::Down, KeyModifiers::NONE);
    press(&mut application, KeyCode::Enter, KeyModifiers::NONE);
    let still = rendered_application_rows_at(&application, 140, 18).join("\n");
    assert!(
        still.contains("Max [unavailable]")
            && still.contains("Not on your plan. The most capable models."),
        "choosing a disabled tier leaves the choices standing: {still}"
    );
}

#[test]
fn option_and_choice_descriptions_wrap_beside_the_list_and_copy_as_written() {
    use suru::protocol::{EffectiveSettings, SidebarVisibility, TextSelectionCopy};

    const OPTION_DESCRIPTION: &str = "Choose how the Model balances capability and response time. This explanation is \
         intentionally longer than a single list row so its concluding guidance stays readable.";
    const CHOICE_DESCRIPTION: &str = "Use the most capable configuration for demanding work, with additional processing \
         before responding. This longer explanation keeps its final recommendation visible.";

    for (provider, role) in [
        ("copilot", ModelOptionRole::RoutingTier),
        ("copilot", ModelOptionRole::ReasoningEffort),
        ("codex", ModelOptionRole::ReasoningEffort),
        ("claude", ModelOptionRole::ReasoningEffort),
    ] {
        let mut model = model_descriptor(
            provider,
            "configurable",
            "Configurable Model",
            true,
            ModelAvailability::Available,
        );
        model.options.push(ModelOptionDescriptor {
            id: ModelOptionId::new("configuration"),
            label: "Configuration".to_owned(),
            description: Some(OPTION_DESCRIPTION.to_owned()),
            role,
            kind: ModelOptionKind::Select {
                choices: vec![
                    ModelOptionChoice {
                        id: ModelOptionChoiceId::new("standard"),
                        label: "Standard".to_owned(),
                        description: Some(CHOICE_DESCRIPTION.to_owned()),
                        availability: ModelAvailability::Available,
                    },
                    ModelOptionChoice {
                        id: ModelOptionChoiceId::new("alternate"),
                        label: "Alternate".to_owned(),
                        description: Some("A different configuration.".to_owned()),
                        availability: ModelAvailability::Available,
                    },
                ],
                default: ModelOptionChoiceId::new("standard"),
            },
        });
        let mut application = landing_on_model_with_options(model);
        let mut settings = EffectiveSettings::default();
        settings.sidebar.initial_visibility = SidebarVisibility::Hidden;
        settings.text_selection.copy = TextSelectionCopy::Release;
        deliver_settings(&mut application, settings);

        let buffer = rendered_application_buffer(&application, 140, 30);
        let option = text_position(&buffer, "› Configuration");
        let start = text_position(&buffer, "Choose how");
        let tail = text_position(&buffer, "readable.");
        assert!(start.0 > option.0 && start.1 < tail.1);
        assert_eq!(
            super::selection::drag(
                &mut application,
                start,
                (tail.0 + "readable.".len() as u16 - 1, tail.1),
            ),
            ApplicationTransition::CopyToClipboard(OPTION_DESCRIPTION.into()),
        );

        press(&mut application, KeyCode::Enter, KeyModifiers::NONE);
        let buffer = rendered_application_buffer(&application, 140, 30);
        let choice = text_position(&buffer, "› Standard [current]");
        let start = text_position(&buffer, "Use the most");
        let tail = text_position(&buffer, "visible.");
        assert!(start.0 > choice.0 && start.1 < tail.1);
        assert_eq!(
            super::selection::drag(
                &mut application,
                start,
                (tail.0 + "visible.".len() as u16 - 1, tail.1),
            ),
            ApplicationTransition::CopyToClipboard(CHOICE_DESCRIPTION.into()),
        );

        press(&mut application, KeyCode::Down, KeyModifiers::NONE);
        let moved = rendered_application_rows_at(&application, 140, 30).join("\n");
        assert!(moved.contains("A different configuration."));
        assert!(!moved.contains("Use the most capable"));
        press(&mut application, KeyCode::Enter, KeyModifiers::NONE);
        assert!(
            rendered_application_rows_at(&application, 140, 30)
                .join("\n")
                .contains("Configuration · Alternate")
        );
    }
}

#[test]
fn split_options_mark_overflow_and_keep_compact_choices_on_narrow_terminals() {
    let mut model = routed_model();
    model.options[0].description = Some("A lengthy explanation of routing choices. ".repeat(80));
    let mut application = landing_on_model_with_options(model);
    let split = rendered_application_rows_at(&application, 140, 12);
    assert!(split.iter().any(|row| row.contains('…')));
    assert!(split.iter().any(|row| row.contains("› Routing tier")));
    assert!(split.iter().any(|row| row.contains("Confirm")));

    press(&mut application, KeyCode::Enter, KeyModifiers::NONE);
    for (width, height) in [(80, 14), (103, 14), (104, 14), (140, 5)] {
        let rows = rendered_application_rows_at(&application, width, height).join("\n");
        assert!(
            rows.contains("› Standard [current]"),
            "{width}x{height}: {rows}"
        );
    }
    let narrow = rendered_application_rows_at(&application, 80, 14).join("\n");
    assert!(narrow.contains("Standard [current] · Versatile models."));
    let wide = rendered_application_rows_at(&application, 104, 14).join("\n");
    assert!(!wide.contains("Standard [current] · Versatile models."));
    assert!(wide.contains("Versatile models."));
}

/// The picker shows Copilot's default as the choice in force, and taking it stores no tier, because
/// no tier already means Copilot's default. The Selection still reads as what the picker showed.
#[test]
fn taking_the_default_routing_tier_stores_none_and_reads_as_the_picker_showed_it() {
    let mut application = landing_on_the_routed_model();
    let options = rendered_application_rows_at(&application, 140, 18).join("\n");
    assert!(options.contains("Routing tier · Standard"), "{options}");

    let ApplicationTransition::ConfirmLandingAgentSelection(confirmed) =
        press(&mut application, KeyCode::Enter, KeyModifiers::CONTROL)
    else {
        panic!("the Landing confirms its Selection");
    };

    assert_eq!(confirmed.routing_tier(), None);
    assert_eq!(confirmed.options, Vec::new());
    let summary = rendered_application_rows_at(&application, 140, 16).join("\n");
    assert!(summary.contains("Copilot · Auto · Standard"), "{summary}");
}

/// High and back again: the picker marks what is in force, and the way back stores no tier, which is
/// how the Session learns to clear the one Copilot holds.
#[test]
fn going_back_to_the_default_routing_tier_stores_none() {
    let mut application = landing_on_the_routed_model();
    press(&mut application, KeyCode::Enter, KeyModifiers::NONE);
    let choices = rendered_application_rows_at(&application, 140, 18).join("\n");
    assert!(choices.contains("Standard [current]"), "{choices}");
    press(&mut application, KeyCode::Down, KeyModifiers::NONE);
    press(&mut application, KeyCode::Enter, KeyModifiers::NONE);
    let options = rendered_application_rows_at(&application, 140, 18).join("\n");
    assert!(options.contains("Routing tier · High"), "{options}");

    press(&mut application, KeyCode::Enter, KeyModifiers::NONE);
    press(&mut application, KeyCode::Up, KeyModifiers::NONE);
    press(&mut application, KeyCode::Enter, KeyModifiers::NONE);
    let options = rendered_application_rows_at(&application, 140, 18).join("\n");
    assert!(options.contains("Routing tier · Standard"), "{options}");

    let ApplicationTransition::ConfirmLandingAgentSelection(confirmed) =
        press(&mut application, KeyCode::Enter, KeyModifiers::CONTROL)
    else {
        panic!("the Landing confirms its Selection");
    };
    assert_eq!(confirmed.options, Vec::new());
}

#[test]
fn a_chosen_routing_tier_stands_in_the_selection_summary() {
    let mut application = landing_on_the_routed_model();
    press(&mut application, KeyCode::Enter, KeyModifiers::NONE);
    press(&mut application, KeyCode::Down, KeyModifiers::NONE);
    press(&mut application, KeyCode::Enter, KeyModifiers::NONE);
    let ApplicationTransition::ConfirmLandingAgentSelection(confirmed) =
        press(&mut application, KeyCode::Enter, KeyModifiers::CONTROL)
    else {
        panic!("the Landing confirms its Selection");
    };

    assert_eq!(
        confirmed.routing_tier().map(ToString::to_string).as_deref(),
        Some("hydrafusion")
    );
    let summary = rendered_application_rows_at(&application, 140, 16).join("\n");
    assert!(
        summary.contains("Copilot · Auto · High"),
        "the footer names the tier: {summary}"
    );
}

/// A Landing's Selection names a tier the catalog no longer offers, with the Setting turned off: the
/// Agent reads as plain Auto, and it is the next Turn that tells the reader so.
#[test]
fn a_selection_naming_a_routing_tier_no_longer_on_offer_reads_as_its_model() {
    let mut application = landing_on_the_routed_model();
    press(&mut application, KeyCode::Enter, KeyModifiers::NONE);
    press(&mut application, KeyCode::Down, KeyModifiers::NONE);
    press(&mut application, KeyCode::Enter, KeyModifiers::NONE);
    press(&mut application, KeyCode::Enter, KeyModifiers::CONTROL);
    let mut plain = routed_model();
    plain.options.clear();
    application
        .handle_event(ApplicationEvent::Managed(
            suru::managed_client::ManagedEvent::ModelCatalog(ModelCatalog {
                providers: vec![ProviderModelCatalog {
                    provider: ProviderId::new("copilot"),
                    display_name: "Copilot".to_owned(),
                    models: vec![plain],
                    status: ProviderCatalogStatus::Fresh,
                }],
            }),
        ))
        .expect("take in the catalog as the Setting left it");

    let summary = rendered_application_rows_at(&application, 140, 16).join("\n");
    assert!(summary.contains("Copilot · Auto"), "{summary}");
    assert!(!summary.contains("High"), "{summary}");
    assert!(!summary.contains("hydrafusion"), "{summary}");
}

#[test]
fn the_pointer_walks_options_and_their_choices_and_pressing_acts_as_enter_does() {
    let mut application = landing_on_the_tiered_model();
    assert!(application.wants_pointer_motion());
    let cell = |application: &Application, text: &str| {
        text_position(&rendered_application_buffer(application, 80, 15), text)
    };
    let focused = |application: &Application| {
        rendered_application_rows(application)
            .into_iter()
            .find(|row| row.contains('›'))
            .expect("a row is focused")
    };

    let context = cell(&application, "Context · Default");
    assert!(point_at(&mut application, context));
    assert!(focused(&application).contains("Context · Default"));

    left_click_at(&mut application, context);
    assert!(
        rendered_application_rows(&application)
            .join("\n")
            .contains("Context Choices")
    );
    let long = cell(&application, "Long context");
    assert!(point_at(&mut application, long));
    assert!(focused(&application).contains("Long context"));
    left_click_at(&mut application, long);
    assert!(
        rendered_application_rows(&application)
            .join("\n")
            .contains("Context · Long context")
    );

    let confirm = cell(&application, "Confirm");
    let ApplicationTransition::ConfirmLandingAgentSelection(confirmed) =
        left_click_at(&mut application, confirm)
    else {
        panic!("pressing Confirm confirms the staged Agent Selection");
    };
    assert!(confirmed.options.contains(&ModelOptionSelection {
        id: ModelOptionId::new("context_tier"),
        value: ModelOptionValue::Select {
            choice: ModelOptionChoiceId::new("long_context"),
        },
    }));
}
