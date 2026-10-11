//! The same independent expectations replayed through GUI and TUI input handlers.
//! Included only by adapter tests; never linked into either application.
#[derive(Clone, Copy, Debug)]
pub enum Step {
    Inspect,
    ApplyRight,
    ApplyLeft,
    Undo,
    Redo,
    Save,
    Discard,
    Reload,
    SwitchRoundTrip,
    External(&'static str),
    RefusedSave,
}

pub struct Case {
    pub name: &'static str,
    pub left: &'static str,
    pub right: &'static str,
    pub onto_left: &'static str,
    pub onto_right: &'static str,
}

pub const CASES: &[Case] = &[
    Case {
        name: "replacement",
        left: "old\n",
        right: "new\n",
        onto_left: "new\n",
        onto_right: "old\n",
    },
    Case {
        name: "insert-into-empty",
        left: "one\ntwo\n",
        right: "",
        onto_left: "",
        onto_right: "one\ntwo\n",
    },
    Case {
        name: "delete-to-empty",
        left: "",
        right: "one\ntwo\n",
        onto_left: "one\ntwo\n",
        onto_right: "",
    },
    Case {
        name: "no-final-newline",
        left: "old",
        right: "new",
        onto_left: "new",
        onto_right: "old",
    },
    Case {
        name: "different-final-newline",
        left: "old\n",
        right: "new",
        onto_left: "new",
        onto_right: "old\n",
    },
    Case {
        name: "unicode-tabs",
        left: "\t旧 café e\u{301} 👩‍💻\n",
        right: "\t新 κόσμος 🦀\n",
        onto_left: "\t新 κόσμος 🦀\n",
        onto_right: "\t旧 café e\u{301} 👩‍💻\n",
    },
    Case {
        name: "crlf",
        left: "old\r\n",
        right: "new\r\n",
        onto_left: "new\r\n",
        onto_right: "old\r\n",
    },
    Case {
        name: "different-eol",
        left: "old\n",
        right: "new\r\n",
        onto_left: "new\n",
        onto_right: "old\r\n",
    },
];

pub const MERGE_CHOICES: &[(&str, &str)] = &[
    ("right", "needle ours\n"),
    ("left", "needle theirs\n"),
    ("down", "needle ours\nneedle theirs\n"),
    ("backspace", "needle base\n"),
];

#[derive(Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub documents: [String; 2],
    pub disk: [Vec<u8>; 2],
    pub dirty: bool,
    pub last_applied_active: Option<bool>,
}

/// Reuse each live session, including after failed writes and discarded drafts.
/// Expectations are literal fixture bytes, independent of the diff/write engine.
pub fn replay(case: &Case, mut input: impl FnMut(Step) -> Snapshot) {
    const EXTERNAL: &str = "external writer\n";
    for round in 0..3 {
        let original = [case.left, case.right];
        let draft = [case.left, case.onto_right];
        let checkpoints = [
            (Step::Inspect, original, original, false),
            (Step::ApplyRight, draft, original, true),
            (Step::SwitchRoundTrip, draft, original, true),
            (Step::Undo, original, original, false),
            (Step::Redo, draft, original, true),
            (Step::Save, draft, draft, false),
            (Step::Reload, draft, draft, false),
            (Step::Undo, original, draft, true),
            (Step::Save, original, original, false),
            (
                Step::ApplyLeft,
                [case.onto_left, case.right],
                original,
                true,
            ),
            (Step::Discard, original, original, false),
            (Step::ApplyRight, draft, original, true),
            (Step::External(EXTERNAL), draft, [case.left, EXTERNAL], true),
            (Step::RefusedSave, draft, [case.left, EXTERNAL], true),
            (Step::Reload, draft, [case.left, EXTERNAL], true),
            (
                Step::Discard,
                [case.left, EXTERNAL],
                [case.left, EXTERNAL],
                false,
            ),
            (
                Step::External(case.right),
                [case.left, EXTERNAL],
                original,
                false,
            ),
            (Step::Reload, original, original, false),
        ];
        let history = [
            None,
            Some(true),
            Some(true),
            Some(false),
            Some(true),
            Some(true),
            Some(true),
            Some(false),
            Some(false),
            Some(true),
            Some(false),
            Some(true),
            Some(true),
            Some(true),
            Some(true),
            None,
            None,
            None,
        ];
        for ((step, documents, disk, dirty), last_applied_active) in
            checkpoints.into_iter().zip(history)
        {
            assert_eq!(
                input(step),
                Snapshot {
                    documents: documents.map(str::to_owned),
                    disk: disk.map(|text| text.as_bytes().to_vec()),
                    dirty,
                    last_applied_active,
                },
                "{} round {round}, {step:?}",
                case.name,
            );
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum RandomAction {
    Edit(Step),
    FindBlockedSave,
    Resize,
}

#[derive(Clone, Default)]
struct ModelHistory {
    active: Vec<bool>,
    undo: Vec<usize>,
    redo: Vec<usize>,
}

/// A deliberately small independent model: one literal hunk, right-side edits,
/// compact command history, and a saved checkpoint. No production helpers.
/// LENZ_FLOW_SEED reproduces a failing sequence reported with its full prefix.
pub fn randomized(case: &Case, seed: u64, mut input: impl FnMut(RandomAction) -> Snapshot) {
    eprintln!("randomized case {}, LENZ_FLOW_SEED={seed}", case.name);
    let (mut current, mut disk) = (false, false);
    let mut history = ModelHistory::default();
    let mut saved = ModelHistory::default();
    let mut random = seed;
    let mut transcript = Vec::new();
    for _ in 0..64 {
        let mut choices = vec![
            RandomAction::Edit(Step::Inspect),
            RandomAction::Edit(Step::Reload),
            RandomAction::Edit(Step::SwitchRoundTrip),
            RandomAction::FindBlockedSave,
            RandomAction::Resize,
        ];
        if !current {
            choices.push(RandomAction::Edit(Step::ApplyRight));
        }
        if !history.undo.is_empty() {
            choices.push(RandomAction::Edit(Step::Undo));
        }
        if !history.redo.is_empty() {
            choices.push(RandomAction::Edit(Step::Redo));
        }
        if current != disk {
            choices.extend([
                RandomAction::Edit(Step::Save),
                RandomAction::Edit(Step::Discard),
            ]);
        }
        random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
        let action = choices[(random >> 32) as usize % choices.len()];
        match action {
            RandomAction::Edit(Step::ApplyRight) => {
                let id = history.active.len();
                history.active.push(true);
                history.undo.push(id);
                history.redo.clear();
                current = true;
            }
            RandomAction::Edit(Step::Undo) => {
                let id = history.undo.pop().unwrap();
                history.active[id] = false;
                history.redo.push(id);
                current = false;
            }
            RandomAction::Edit(Step::Redo) => {
                let id = history.redo.pop().unwrap();
                history.active[id] = true;
                history.undo.push(id);
                current = true;
            }
            RandomAction::Edit(Step::Save) => {
                disk = current;
                saved = history.clone();
            }
            RandomAction::Edit(Step::Discard) => {
                current = disk;
                history = saved.clone();
            }
            RandomAction::Edit(Step::Inspect | Step::Reload | Step::SwitchRoundTrip)
            | RandomAction::FindBlockedSave
            | RandomAction::Resize => {}
            RandomAction::Edit(step) => unreachable!("not generated: {step:?}"),
        }
        transcript.push(action);
        let actual = input(action);
        assert_eq!(
            actual,
            Snapshot {
                documents: [
                    case.left.to_owned(),
                    if current { case.onto_right } else { case.right }.to_owned()
                ],
                disk: [
                    case.left.as_bytes().to_vec(),
                    if disk { case.onto_right } else { case.right }
                        .as_bytes()
                        .to_vec()
                ],
                dirty: current != disk,
                last_applied_active: history.active.last().copied(),
            },
            "{} seed {seed}, step {}: {transcript:?}",
            case.name,
            transcript.len()
        );
    }
}

pub fn seeds() -> Vec<u64> {
    std::env::var("LENZ_FLOW_SEED").map_or_else(
        |_| vec![1, 7, 42, 0x5eed_cafe],
        |value| {
            vec![
                value
                    .parse()
                    .expect("LENZ_FLOW_SEED must be an unsigned integer"),
            ]
        },
    )
}
