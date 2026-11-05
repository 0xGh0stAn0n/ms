//! The cast's voices made on an ElevenLabs account, against a stand-in
//! server on this machine through the same `curl` the real calls use: a
//! voice designed from the character's description and sample, its first
//! take kept with the character's name and label, and found again (never
//! made twice); a voice-design model the account can't use giving way to
//! the next; nothing designed on an account with no room for a voice, or
//! kept without credits or with a refused key; a save whose answer was lost
//! found on the account rather than made again; and the maker handing back
//! what came of it.

use std::net::TcpListener;
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use ms::ai::AiError;
use ms::ai::cast::{Cast, Maker};
use ms::ai::eleven::Eleven;
use ms::phone::http::{Conn, Response};
use serde_json::{Value, json};

const KEY: &str = "sk_0123456789abcdef0123456789abcdef0123456789abcdef";

/// How the stand-in account behaves.
#[derive(Clone, Copy, Default)]
struct Account {
    /// The first voice-design model is not one it can use.
    no_first_model: bool,
    /// Every voice slot is taken.
    full: bool,
    /// A voice is kept, but the answer saying so never comes back.
    answer_lost: bool,
    /// No credits left to design with.
    no_credits: bool,
    /// The key may not read the plan.
    plan_hidden: bool,
}

/// A fake ElevenLabs: every request kept (path, output format, body), the
/// voices it made listed back.
fn fake(cast: &Cast, account: Account) -> (String, Arc<Mutex<Vec<Value>>>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let made: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let first_model = cast.design_models[0].clone();
    let log = Arc::clone(&seen);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let (log, made, first_model) =
                (Arc::clone(&log), Arc::clone(&made), first_model.clone());
            std::thread::spawn(move || {
                let mut conn = Conn::new(stream);
                while let Ok(Some(req)) = conn.read_request(1 << 20) {
                    let body: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
                    log.lock().unwrap().push(json!({
                        "path": req.path,
                        "format": req.param("output_format"),
                        "body": body,
                    }));
                    let response = if req.header("xi-api-key") != Some(KEY) {
                        Response::json(
                            401,
                            &json!({"detail": {"status": "invalid_api_key", "message": "Invalid API key"}}),
                        )
                    } else {
                        match req.path.as_str() {
                            "/v1/user/subscription" if account.plan_hidden => Response::json(
                                403,
                                &json!({"detail": {"status": "missing_permissions", "message": "The API key is missing the permission user_read."}}),
                            ),
                            "/v1/user/subscription" => {
                                let limit = 30;
                                let used = if account.full {
                                    limit
                                } else {
                                    made.lock().unwrap().len()
                                };
                                Response::json(
                                    200,
                                    &json!({"tier": "creator", "voice_slots_used": used, "voice_limit": limit}),
                                )
                            }
                            "/v1/text-to-voice/design" if account.no_credits => Response::json(
                                401,
                                &json!({"detail": {"status": "quota_exceeded", "message": "This request exceeds your quota."}}),
                            ),
                            "/v1/text-to-voice/design"
                                if account.no_first_model
                                    && body["model_id"] == first_model.as_str() =>
                            {
                                Response::json(
                                    422,
                                    &json!({"detail": [{"loc": ["body", "model_id"], "msg": "Input should be another model", "type": "enum"}]}),
                                )
                            }
                            "/v1/text-to-voice/design" => Response::json(
                                200,
                                &json!({
                                    "previews": [
                                        {"generated_voice_id": "", "audio_base_64": "", "media_type": "audio/mpeg", "duration_secs": 0.0},
                                        {"generated_voice_id": "take-1", "audio_base_64": "", "media_type": "audio/mpeg", "duration_secs": 9.5},
                                        {"generated_voice_id": "take-2", "audio_base_64": "", "media_type": "audio/mpeg", "duration_secs": 9.1},
                                    ],
                                    "text": body["text"],
                                }),
                            ),
                            "/v1/text-to-voice" => {
                                let voice = json!({
                                    "voice_id": format!("made-{}", made.lock().unwrap().len() + 1),
                                    "name": body["voice_name"],
                                    "labels": body["labels"],
                                    "category": "generated",
                                });
                                made.lock().unwrap().push(voice.clone());
                                if account.answer_lost {
                                    Response::json(500, &json!({"detail": "Internal Server Error"}))
                                } else {
                                    Response::json(200, &voice)
                                }
                            }
                            "/v1/voices" => {
                                Response::json(200, &json!({"voices": *made.lock().unwrap()}))
                            }
                            _ => Response::json(404, &json!({"detail": "Not Found"})),
                        }
                    };
                    if conn.write_response(&response, !req.wants_close()).is_err() {
                        return;
                    }
                }
            });
        }
    });
    (format!("http://127.0.0.1:{port}/v1"), seen)
}

fn have_curl() -> bool {
    let here = std::process::Command::new(if cfg!(windows) { "curl.exe" } else { "curl" })
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success());
    if !here {
        eprintln!("curl is not installed here; skipped");
    }
    here
}

/// The requests to `path` so far.
fn asked(seen: &Mutex<Vec<Value>>, path: &str) -> Vec<Value> {
    seen.lock()
        .unwrap()
        .iter()
        .filter(|r| r["path"] == path)
        .cloned()
        .collect()
}

const DESIGN: &str = "/v1/text-to-voice/design";
const SAVE: &str = "/v1/text-to-voice";

#[test]
fn a_characters_voice_is_designed_from_its_description_and_kept_with_its_label() {
    if !have_curl() {
        return;
    }
    let cast = Cast::built_in().unwrap();
    let (base, seen) = fake(&cast, Account::default());
    let eleven = Eleven::new(KEY, &base);
    let character = &cast.characters[0];
    assert!(
        cast.voice_of(character, &eleven.voices().unwrap())
            .is_none()
    );

    let voice = cast.make(character, &eleven).unwrap();
    // Designed from how it sounds, saying its sample, with the first
    // model, the takes sent back small.
    let design = asked(&seen, DESIGN);
    assert_eq!(design.len(), 1);
    assert_eq!(
        design[0]["body"]["voice_description"],
        character.description.as_str()
    );
    assert_eq!(design[0]["body"]["text"], character.sample.as_str());
    assert_eq!(
        design[0]["body"]["model_id"],
        cast.design_models[0].as_str()
    );
    assert!(design[0]["format"].as_str().is_some_and(|f| !f.is_empty()));
    // The first take with an id kept, under the character's name and with
    // its label.
    let save = asked(&seen, SAVE);
    assert_eq!(save.len(), 1);
    assert_eq!(save[0]["body"]["generated_voice_id"], "take-1");
    assert_eq!(save[0]["body"]["voice_name"], cast.voice_name_of(character));
    assert_eq!(
        save[0]["body"]["voice_description"],
        character.description.as_str()
    );
    assert_eq!(
        save[0]["body"]["labels"],
        json!({cast.label.clone(): character.id.clone()})
    );
    assert_eq!(voice.name, cast.voice_name_of(character));

    // On the account now: found by its label, and never made twice.
    let listed = eleven.voices().unwrap();
    assert_eq!(
        cast.voice_of(character, &listed).map(|v| &v.id),
        Some(&voice.id)
    );
    assert!(cast.voice_of(&cast.characters[1], &listed).is_none());
    assert_eq!(cast.make(character, &eleven).unwrap().id, voice.id);
    assert_eq!(
        (asked(&seen, DESIGN).len(), asked(&seen, SAVE).len()),
        (1, 1)
    );
}

#[test]
fn a_model_the_account_cant_use_gives_way_to_the_next() {
    if !have_curl() {
        return;
    }
    let cast = Cast::built_in().unwrap();
    assert!(cast.design_models.len() > 1, "a model to fall back on");
    let account = Account {
        no_first_model: true,
        ..Account::default()
    };
    let (base, seen) = fake(&cast, account);
    let voice = cast
        .make(&cast.characters[1], &Eleven::new(KEY, &base))
        .unwrap();
    let models: Vec<Value> = asked(&seen, DESIGN)
        .iter()
        .map(|r| r["body"]["model_id"].clone())
        .collect();
    assert_eq!(
        models,
        [json!(cast.design_models[0]), json!(cast.design_models[1])]
    );
    assert_eq!(asked(&seen, SAVE).len(), 1);
    assert!(!voice.id.is_empty());
}

#[test]
fn nothing_is_designed_on_an_account_with_no_room_for_a_voice() {
    if !have_curl() {
        return;
    }
    let cast = Cast::built_in().unwrap();
    let account = Account {
        full: true,
        ..Account::default()
    };
    let (base, seen) = fake(&cast, account);
    let error = cast
        .make(&cast.characters[0], &Eleven::new(KEY, &base))
        .unwrap_err();
    assert!(matches!(error, AiError::Unsupported(_)), "{error:?}");
    assert!(asked(&seen, DESIGN).is_empty() && asked(&seen, SAVE).is_empty());

    // An account that won't say gets the benefit of the doubt.
    let account = Account {
        plan_hidden: true,
        ..Account::default()
    };
    let (base, seen) = fake(&cast, account);
    cast.make(&cast.characters[0], &Eleven::new(KEY, &base))
        .unwrap();
    assert_eq!(asked(&seen, SAVE).len(), 1);
}

#[test]
fn a_voice_kept_whose_answer_was_lost_is_found_not_made_again() {
    if !have_curl() {
        return;
    }
    let cast = Cast::built_in().unwrap();
    let account = Account {
        answer_lost: true,
        ..Account::default()
    };
    let (base, seen) = fake(&cast, account);
    let eleven = Eleven::new(KEY, &base);
    let character = &cast.characters[2];
    let voice = cast.make(character, &eleven).unwrap();
    assert_eq!(
        cast.voice_of(character, &eleven.voices().unwrap()),
        Some(&voice)
    );
    assert_eq!(
        (asked(&seen, DESIGN).len(), asked(&seen, SAVE).len()),
        (1, 1)
    );
}

#[test]
fn without_credits_or_with_a_refused_key_nothing_is_kept() {
    if !have_curl() {
        return;
    }
    let cast = Cast::built_in().unwrap();
    let account = Account {
        no_credits: true,
        ..Account::default()
    };
    let (base, seen) = fake(&cast, account);
    let error = cast
        .make(&cast.characters[0], &Eleven::new(KEY, &base))
        .unwrap_err();
    assert!(matches!(error, AiError::Http(401, _)), "{error:?}");
    assert_eq!(asked(&seen, DESIGN).len(), 1, "one model is enough to know");
    assert!(asked(&seen, SAVE).is_empty());

    let (base, seen) = fake(&cast, Account::default());
    let error = cast
        .make(&cast.characters[0], &Eleven::new("sk_refused", &base))
        .unwrap_err();
    assert!(matches!(error, AiError::Http(401, _)), "{error:?}");
    assert!(asked(&seen, DESIGN).is_empty() && asked(&seen, SAVE).is_empty());
}

#[test]
fn the_maker_hands_back_what_came_of_it() {
    if !have_curl() {
        return;
    }
    let cast = Arc::new(Cast::built_in().unwrap());
    let (base, _) = fake(&cast, Account::default());
    let character = cast.characters[3].clone();
    let (done, made) = mpsc::channel();
    for (key, works) in [(KEY, true), ("sk_refused", false)] {
        Maker::new(key, &base, Arc::clone(&cast), done.clone())
            .start(character.clone())
            .unwrap();
        let made = made.recv_timeout(Duration::from_secs(30)).unwrap();
        assert_eq!(made.character, character);
        assert_eq!(made.voice.is_ok(), works, "{:?}", made.voice);
    }
}
