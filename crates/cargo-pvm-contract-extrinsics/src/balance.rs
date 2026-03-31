use std::{fmt::Display, str::FromStr};

use anyhow::{Context, Result, anyhow};
use rust_decimal::{Decimal, prelude::FromPrimitive};
use serde_json::json;
use subxt::{
    Config,
    backend::{legacy::LegacyRpcMethods, rpc::RpcClient},
};
use url::Url;

use crate::url_to_string;

/// Represents different formats of a balance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BalanceVariant<Balance> {
    /// Default format: no symbol, no token_decimals.
    Default(Balance),
    /// Denominated format: symbol and token_decimals are present.
    Denominated(DenominatedBalance),
}

#[derive(Debug, Clone)]
pub struct TokenMetadata {
    /// Number of token_decimals used for denomination.
    pub token_decimals: usize,
    /// Token symbol.
    pub symbol: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DenominatedBalance {
    value: Decimal,
    unit: UnitPrefix,
    symbol: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnitPrefix {
    Giga,
    Mega,
    Kilo,
    One,
    Milli,
    Micro,
    Nano,
}

impl TokenMetadata {
    /// Query [TokenMetadata] through the node's RPC.
    pub async fn query<C: Config>(url: &Url) -> Result<Self> {
        let rpc_cli = RpcClient::from_url(url_to_string(url)).await?;
        let rpc = LegacyRpcMethods::<C>::new(rpc_cli);
        let sys_props = rpc.system_properties().await?;

        let default_decimals = json!(12);
        let default_units = json!("UNIT");
        let token_decimals = sys_props
            .get("tokenDecimals")
            .unwrap_or(&default_decimals)
            .as_u64()
            .or_else(|| {
                sys_props
                    .get("tokenDecimals")
                    .and_then(|v| v.as_array()?.first()?.as_u64())
            })
            .context("error converting decimal to u64")? as usize;
        let symbol = sys_props
            .get("tokenSymbol")
            .unwrap_or(&default_units)
            .as_str()
            .context("error converting symbol to string")?;
        Ok(Self {
            token_decimals,
            symbol: symbol.to_string(),
        })
    }
}

impl<Balance> FromStr for BalanceVariant<Balance>
where
    Balance: FromStr,
{
    type Err = anyhow::Error;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let input = input.replace('_', "");
        let result = match input.parse::<Balance>() {
            Ok(balance) => BalanceVariant::Default(balance),
            Err(_) => BalanceVariant::Denominated(DenominatedBalance::from_str(&input)?),
        };
        Ok(result)
    }
}

impl FromStr for DenominatedBalance {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let symbols =
            value.trim_start_matches(|ch: char| ch.is_numeric() || ch == '.' || ch == ',');
        let unit_char = symbols
            .chars()
            .next()
            .context("no units or symbols present")?;
        let unit: UnitPrefix = match unit_char {
            'G' => UnitPrefix::Giga,
            'M' => UnitPrefix::Mega,
            'k' => UnitPrefix::Kilo,
            'm' => UnitPrefix::Milli,
            '\u{3bc}' => UnitPrefix::Micro,
            'n' => UnitPrefix::Nano,
            _ => UnitPrefix::One,
        };
        let symbol = if unit != UnitPrefix::One {
            let (start, _) = symbols
                .char_indices()
                .nth(1)
                .context("cannot find the first char's index")?;
            symbols[start..].to_string()
        } else {
            String::new()
        };
        let value = value.trim_end_matches(|ch: char| ch.is_alphabetic());
        let value = Decimal::from_str_exact(value)
            .context("Error while parsing the value.")?
            .normalize();
        Ok(Self {
            value,
            unit,
            symbol,
        })
    }
}

impl<Balance> BalanceVariant<Balance>
where
    Balance: From<u128> + Clone,
{
    /// Converts BalanceVariant into Balance.
    pub fn denominate_balance(&self, token_metadata: &TokenMetadata) -> Result<Balance> {
        match self {
            BalanceVariant::Default(balance) => Ok(balance.clone()),
            BalanceVariant::Denominated(den_balance) => {
                let zeros: usize = (token_metadata.token_decimals as isize
                    + match den_balance.unit {
                        UnitPrefix::Giga => 9,
                        UnitPrefix::Mega => 6,
                        UnitPrefix::Kilo => 3,
                        UnitPrefix::One => 0,
                        UnitPrefix::Milli => -3,
                        UnitPrefix::Micro => -6,
                        UnitPrefix::Nano => -9,
                    })
                .try_into()?;
                let multiple = Decimal::from_str_exact(&format!("1{}", "0".repeat(zeros)))?;
                let fract_scale = den_balance.value.fract().scale();
                let mantissa_difference = zeros as isize - fract_scale as isize;
                if mantissa_difference < 0 {
                    return Err(anyhow!(
                        "Given precision of a Balance value is higher than allowed"
                    ));
                }
                let balance: u128 = den_balance
                    .value
                    .checked_mul(multiple)
                    .context("error while converting balance to raw format. Overflow during multiplication!")?
                    .try_into()?;
                Ok(balance.into())
            }
        }
    }

    /// Display token units in a denominated format.
    pub fn from<T: Into<u128>>(value: T, token_metadata: Option<&TokenMetadata>) -> Result<Self> {
        let n: u128 = value.into();

        if let Some(token_metadata) = token_metadata {
            if n == 0 {
                return Ok(BalanceVariant::Denominated(DenominatedBalance {
                    value: Decimal::ZERO,
                    unit: UnitPrefix::One,
                    symbol: token_metadata.symbol.clone(),
                }));
            }

            let number_of_digits = n.to_string().len();

            let giga_units_zeros = token_metadata.token_decimals + 9;
            let mega_units_zeros = token_metadata.token_decimals + 6;
            let kilo_units_zeros = token_metadata.token_decimals + 3;
            let one_unit_zeros = token_metadata.token_decimals;
            let milli_units_zeros = token_metadata.token_decimals.checked_sub(3);
            let micro_units_zeros = token_metadata.token_decimals.checked_sub(6);
            let nano_units_zeros = token_metadata.token_decimals.checked_sub(9);

            let unit: UnitPrefix;
            let zeros: usize;
            if (giga_units_zeros + 1..).contains(&number_of_digits) {
                zeros = giga_units_zeros;
                unit = UnitPrefix::Giga;
            } else if (mega_units_zeros + 1..=giga_units_zeros).contains(&number_of_digits) {
                zeros = mega_units_zeros;
                unit = UnitPrefix::Mega;
            } else if (kilo_units_zeros + 1..=mega_units_zeros).contains(&number_of_digits) {
                zeros = kilo_units_zeros;
                unit = UnitPrefix::Kilo;
            } else if (one_unit_zeros + 1..=kilo_units_zeros).contains(&number_of_digits) {
                zeros = one_unit_zeros;
                unit = UnitPrefix::One;
            } else if let Some(milli_zeros) = milli_units_zeros {
                if (milli_zeros + 1..=one_unit_zeros).contains(&number_of_digits) {
                    zeros = milli_zeros;
                    unit = UnitPrefix::Milli;
                } else if let Some(micro_zeros) = micro_units_zeros {
                    if (micro_zeros + 1..=milli_zeros).contains(&number_of_digits) {
                        zeros = micro_zeros;
                        unit = UnitPrefix::Micro;
                    } else if let Some(nano_zeros) = nano_units_zeros {
                        if (nano_zeros + 1..=micro_zeros).contains(&number_of_digits) {
                            zeros = nano_zeros;
                            unit = UnitPrefix::Nano;
                        } else {
                            return Err(anyhow!("Invalid denomination"));
                        }
                    } else {
                        return Err(anyhow!("Invalid denomination"));
                    }
                } else {
                    return Err(anyhow!("Invalid denomination"));
                }
            } else if let Some(nano_zeros) = nano_units_zeros {
                zeros = nano_zeros;
                unit = UnitPrefix::Nano;
            } else {
                return Err(anyhow!("Invalid denomination"));
            }
            let multiple = Decimal::from_str_exact(&format!("1{}", "0".repeat(zeros)))?;
            let value = Decimal::from_u128(n).context("value can not be converted into decimal")?
                / multiple;

            Ok(BalanceVariant::Denominated(DenominatedBalance {
                value,
                unit,
                symbol: token_metadata.symbol.clone(),
            }))
        } else {
            Ok(BalanceVariant::Default(n.into()))
        }
    }
}

impl<Balance> Display for BalanceVariant<Balance>
where
    Balance: Display + Clone,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BalanceVariant::Default(balance) => f.write_str(&balance.to_string()),
            BalanceVariant::Denominated(input) => f.write_str(&input.to_string()),
        }
    }
}

impl Display for DenominatedBalance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let prefix = match self.unit {
            UnitPrefix::Giga => "G",
            UnitPrefix::Mega => "M",
            UnitPrefix::Kilo => "k",
            UnitPrefix::One => "",
            UnitPrefix::Milli => "m",
            UnitPrefix::Micro => "\u{3bc}",
            UnitPrefix::Nano => "n",
        };
        f.write_fmt(format_args!("{}{}{}", self.value, prefix, self.symbol))
    }
}
