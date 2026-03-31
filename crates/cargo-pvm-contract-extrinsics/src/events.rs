use anyhow::Result;
use std::fmt::Write;
use subxt::{
    Config,
    blocks::ExtrinsicEvents,
    events::StaticEvent,
    ext::{scale_decode, scale_encode},
    utils::{H160, H256},
};

/// A custom event emitted by the contract.
#[derive(
    scale::Decode, scale::Encode, scale_decode::DecodeAsType, scale_encode::EncodeAsType, Debug,
)]
#[decode_as_type(crate_path = "subxt::ext::scale_decode")]
#[encode_as_type(crate_path = "subxt::ext::scale_encode")]
pub struct ContractEmitted {
    /// The contract that emitted the event.
    pub contract: H160,
    /// Data supplied by the contract.
    pub data: Vec<u8>,
    /// A list of topics used to index the event.
    pub topics: Vec<H256>,
}

impl StaticEvent for ContractEmitted {
    const PALLET: &'static str = "Revive";
    const EVENT: &'static str = "ContractEmitted";
}

/// Contract deployed by deployer at the specified address.
#[derive(
    scale::Decode, scale::Encode, scale_decode::DecodeAsType, scale_encode::EncodeAsType, Debug,
)]
#[decode_as_type(crate_path = "subxt::ext::scale_decode")]
#[encode_as_type(crate_path = "subxt::ext::scale_encode")]
pub struct ContractInstantiated {
    /// Address of the deployer.
    pub deployer: H160,
    /// Address where the contract was instantiated to.
    pub contract: H160,
}

impl StaticEvent for ContractInstantiated {
    const PALLET: &'static str = "Revive";
    const EVENT: &'static str = "Instantiated";
}

/// An event produced from invoking a contract extrinsic.
#[derive(serde::Serialize)]
pub struct Event {
    /// name of a pallet
    pub pallet: String,
    /// name of the event
    pub name: String,
    /// event fields as key-value pairs
    pub fields: Vec<(String, String)>,
}

/// Displays events produced from invoking a contract extrinsic.
#[derive(serde::Serialize)]
pub struct DisplayEvents(pub Vec<Event>);

impl DisplayEvents {
    /// Parses events from extrinsic results into a displayable format.
    pub fn from_events<C: Config>(result: &ExtrinsicEvents<C>) -> Result<DisplayEvents> {
        let mut events: Vec<Event> = vec![];

        for event in result.iter() {
            let event = event?;
            let mut event_entry = Event {
                pallet: event.pallet_name().to_string(),
                name: event.variant_name().to_string(),
                fields: vec![],
            };

            // For ContractEmitted events, show the raw hex data
            if <ContractEmitted as StaticEvent>::is_event(event.pallet_name(), event.variant_name())
                && let Some(ce) = event.as_event::<ContractEmitted>().ok().flatten()
            {
                event_entry.fields.push((
                    "contract".to_string(),
                    format!("0x{}", hex::encode(ce.contract.as_bytes())),
                ));
                event_entry
                    .fields
                    .push(("data".to_string(), format!("0x{}", hex::encode(&ce.data))));
                for (i, topic) in ce.topics.iter().enumerate() {
                    event_entry.fields.push((
                        format!("topic[{i}]"),
                        format!("0x{}", hex::encode(topic.as_bytes())),
                    ));
                }
            }

            events.push(event_entry);
        }

        Ok(DisplayEvents(events))
    }

    /// Displays events in a human readable format.
    pub fn display_events(&self) -> Result<String> {
        let mut out = String::new();
        writeln!(out, "Events")?;
        for event in &self.0 {
            writeln!(out, "  {} -> {}", event.pallet, event.name)?;
            for (key, value) in &event.fields {
                writeln!(out, "    {key}: {value}")?;
            }
        }
        Ok(out)
    }

    /// Returns an event result in json format.
    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }
}
