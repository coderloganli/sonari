//! The evaluation set, as data.
//!
//! Fifteen clips, each probing something sonari implements rather than
//! something ElevenLabs implements. Noise, accent and entity coverage are
//! deliberately absent: those measure a recogniser that is already tuned against
//! public benchmarks, and no change we make would move them.
//!
//! The information-bearing words sit in the second half of every pause clip on
//! purpose. A turn cut short then costs an order number or a date, not a
//! greeting — which is what makes the failure worth measuring.

pub struct Clip {
    pub id: String,
    pub reference: String,
    pub shape: Shape,
}

pub enum Shape {
    /// One sentence split by an exact gap.
    Pause {
        first: String,
        gap_ms: u32,
        second: String,
    },
    /// A single word, shorter than `min_utterance_ms`.
    Short { word: String },
    /// A sentence whose tail fades away, as speech naturally does.
    Decay {
        sentence: String,
        fade_ms: u32,
        floor_dbfs: f32,
    },
    /// Nothing unusual. The control group.
    Plain { sentence: String },
    /// No speech at all.
    Silence { ms: u32 },
    /// A cough: loud, brief, not words.
    Burst { ms: u32 },
    /// The wrong format, so the rejection path has something to reject.
    Malformed { sentence: String },
}

fn pause(id: &str, first: &str, gap_ms: u32, second: &str) -> Clip {
    Clip {
        id: id.to_owned(),
        reference: format!("{first} {second}"),
        shape: Shape::Pause {
            first: first.to_owned(),
            gap_ms,
            second: second.to_owned(),
        },
    }
}

fn short(id: &str, word: &str) -> Clip {
    Clip {
        id: id.to_owned(),
        reference: word.to_owned(),
        shape: Shape::Short {
            word: word.to_owned(),
        },
    }
}

fn decay(id: &str, sentence: &str) -> Clip {
    Clip {
        id: id.to_owned(),
        reference: sentence.to_owned(),
        shape: Shape::Decay {
            sentence: sentence.to_owned(),
            fade_ms: 800,
            floor_dbfs: -30.0,
        },
    }
}

fn plain(id: &str, sentence: &str) -> Clip {
    Clip {
        id: id.to_owned(),
        reference: sentence.to_owned(),
        shape: Shape::Plain {
            sentence: sentence.to_owned(),
        },
    }
}

pub fn clips() -> Vec<Clip> {
    vec![
        // The ladder brackets the configured `silence_flush_ms` of 700: two
        // below, two above.
        pause("pause-400", "i'd like a table for", 400, "four people"),
        pause("pause-600", "can you send it to", 600, "the office address"),
        pause("pause-800", "my order number is", 800, "eight two nine one"),
        pause(
            "pause-1200",
            "i want to change my flight to",
            1_200,
            "next tuesday",
        ),
        // All three are shorter than `min_utterance_ms`. `short-two` carries a
        // quantity, so losing it changes an answer rather than dropping an
        // acknowledgement.
        short("short-yeah", "yeah"),
        short("short-no", "no"),
        short("short-two", "two"),
        // Speech naturally fades at the end of a sentence, and what fades is
        // the part that cannot be guessed from context: a surname, a number.
        decay("decay-address", "it's forty two oak street"),
        decay("decay-name", "my name is jonathan whitfield"),
        // The control group. Every other clip is read as a difference from
        // these.
        plain(
            "baseline-booking",
            "hi i'd like to book a table for four people this evening",
        ),
        plain("baseline-question", "what time do you close on sundays"),
        plain(
            "baseline-change",
            "actually can you change that to seven thirty instead",
        ),
        // Nothing should happen. Anything that does is a false trigger, and a
        // transcript from either of these is the system answering itself.
        Clip {
            id: "edge-silence".to_owned(),
            reference: String::new(),
            shape: Shape::Silence { ms: 5_000 },
        },
        Clip {
            id: "edge-cough".to_owned(),
            reference: String::new(),
            shape: Shape::Burst { ms: 150 },
        },
        Clip {
            id: "edge-8khz-stereo".to_owned(),
            reference: "what time do you close on sundays".to_owned(),
            shape: Shape::Malformed {
                sentence: "what time do you close on sundays".to_owned(),
            },
        },
    ]
}
