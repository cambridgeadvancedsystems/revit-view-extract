//! Revit's coherent foot-kilogram-second-ampere values converted by Forge spec.
use serde_json::{Value, json};
pub const FT_TO_M: f64 = 0.3048;
pub const FT2_TO_M2: f64 = FT_TO_M * FT_TO_M;
pub const FT3_TO_M3: f64 = FT_TO_M * FT_TO_M * FT_TO_M;

pub fn round(value: f64, places: u8) -> f64 {
    // Decimal formatting rounds the original binary value, matching Python's
    // round(x, n) at half-way cases without a lossy multiply by 10^n.
    let precision = usize::from(places);
    format!("{value:.precision$}").parse().unwrap_or(value)
}

pub fn fallback_spec(parameter: i64) -> Option<&'static str> {
    Some(match parameter {
        -1140082 | -1140140 | -1140080 => "autodesk.spec.aec.electrical:current",
        -1001105 | -1001108 | -1001109 | -1012828 | -1012829 | -1001951 => {
            "autodesk.spec.aec:length"
        }
        -1001955 => "autodesk.spec.aec:angle",
        -1114230 | -1114229 => "autodesk.spec.aec.electrical:power",
        -1114261 | -1114260 => "autodesk.spec.aec.electrical:powerDensity",
        _ => return None,
    })
}

#[derive(Debug, Clone)]
pub struct Conversion {
    pub value: f64,
    pub unit: &'static str,
    pub secondary: Value,
}

pub fn convert_parameter(spec: Option<&str>, parameter: i64, value: f64) -> Option<Conversion> {
    convert_by_spec(
        spec.filter(|s| !s.is_empty())
            .or_else(|| fallback_spec(parameter))?,
        value,
    )
}

pub fn convert_by_spec(spec: &str, x: f64) -> Option<Conversion> {
    if !x.is_finite() {
        return None;
    }
    let base = if let Some((base, version)) = spec.rsplit_once('-') {
        let parts = version.split('.').collect::<Vec<_>>();
        if parts.len() != 3
            || parts
                .iter()
                .any(|part| part.is_empty() || !part.bytes().all(|c| c.is_ascii_digit()))
        {
            return None;
        }
        base
    } else {
        spec
    };
    let (value, unit, secondary) = match converter(base)? {
        "length" => (
            round(x * FT_TO_M, 6),
            "m",
            json!({"mm":round(x*FT_TO_M*1000.0,2),"in":round(x*12.0,3)}),
        ),
        "area" => (round(x * FT2_TO_M2, 4), "m2", json!({"ft2":round(x,3)})),
        "volume" => (round(x * FT3_TO_M3, 4), "m3", json!({"ft3":round(x,3)})),
        "angle" => (round(x.to_degrees(), 4), "deg", json!({"rad":round(x,6)})),
        "potential" => (
            round(x * FT2_TO_M2, 3),
            "V",
            json!({"kV":round(x*FT2_TO_M2/1000.0,4)}),
        ),
        "apparent_power" => (
            round(x * FT2_TO_M2, 2),
            "VA",
            json!({"kVA":round(x*FT2_TO_M2/1000.0,3)}),
        ),
        "active_power" => (
            round(x * FT2_TO_M2, 2),
            "W",
            json!({"kW":round(x*FT2_TO_M2/1000.0,3)}),
        ),
        "current" => (round(x, 3), "A", json!({})),
        "power_density" => (round(x, 3), "W/m2", json!({"W/ft2":round(x*FT2_TO_M2,3)})),
        "frequency" => (round(x, 3), "Hz", json!({})),
        "demand_factor" => (round(x * 100.0, 2), "%", json!({"ratio":round(x,4)})),
        "illuminance" => (round(x / FT2_TO_M2, 2), "lx", json!({"fc":round(x,2)})),
        "luminous_flux" => (round(x, 2), "lm", json!({})),
        "color_temperature" => (round(x, 1), "K", json!({})),
        "airflow" => (
            round(x * 60.0, 2),
            "CFM",
            json!({"L/s":round(x*FT3_TO_M3*1000.0,2),"m3/h":round(x*FT3_TO_M3*3600.0,2)}),
        ),
        "pressure" => (
            round(x / FT_TO_M, 3),
            "Pa",
            json!({"in_wg":round(x/FT_TO_M/248.84,4)}),
        ),
        "thermal_load" => (
            round(x * FT2_TO_M2, 2),
            "W",
            json!({"kW":round(x*FT2_TO_M2/1000.0,3),"BTU/h":round(x*FT2_TO_M2*3.412141633127942,1)}),
        ),
        "velocity" => (round(x * FT_TO_M, 3), "m/s", json!({"FPM":round(x*60.0,2)})),
        "temperature" => (
            round(x - 273.15, 2),
            "C",
            json!({"F":round((x-273.15)*1.8+32.0,2),"K":round(x,2)}),
        ),
        "number" => (round(x, 6), "dimensionless", json!({})),
        _ => return None,
    };
    if !value.is_finite()
        || secondary
            .as_object()?
            .values()
            .any(|v| !v.as_f64().is_some_and(f64::is_finite))
    {
        return None;
    }
    Some(Conversion {
        value,
        unit,
        secondary,
    })
}

fn converter(spec: &str) -> Option<&'static str> {
    Some(match spec {
        "autodesk.spec.aec:length"
        | "autodesk.spec.aec:sheetLength"
        | "autodesk.spec.aec:decimalSheetLength"
        | "autodesk.spec.aec.electrical:conduitSize"
        | "autodesk.spec.aec.electrical:cableTraySize"
        | "autodesk.spec.aec.electrical:wireDiameter"
        | "autodesk.spec.aec.hvac:ductSize"
        | "autodesk.spec.aec.hvac:ductInsulationThickness"
        | "autodesk.spec.aec.hvac:ductLiningThickness"
        | "autodesk.spec.aec.hvac:roughness"
        | "autodesk.spec.aec.piping:pipeSize"
        | "autodesk.spec.aec.piping:pipeDimension"
        | "autodesk.spec.aec.piping:pipeInsulationThickness" => "length",
        "autodesk.spec.aec:area" => "area",
        "autodesk.spec.aec:volume" | "autodesk.spec.aec.piping:volume" => "volume",
        "autodesk.spec.aec:angle" => "angle",
        "autodesk.spec.aec.electrical:potential" => "potential",
        "autodesk.spec.aec.electrical:apparentPower" => "apparent_power",
        "autodesk.spec.aec.electrical:power" | "autodesk.spec.aec.electrical:wattage" => {
            "active_power"
        }
        "autodesk.spec.aec.electrical:current" => "current",
        "autodesk.spec.aec.electrical:powerDensity"
        | "autodesk.spec.aec.electrical:apparentPowerDensity"
        | "autodesk.spec.aec.hvac:powerDensity" => "power_density",
        "autodesk.spec.aec.electrical:frequency" => "frequency",
        "autodesk.spec.aec.electrical:demandFactor" => "demand_factor",
        "autodesk.spec.aec.electrical:illuminance" => "illuminance",
        "autodesk.spec.aec.electrical:luminousFlux" => "luminous_flux",
        "autodesk.spec.aec.electrical:colorTemperature" => "color_temperature",
        "autodesk.spec.aec.hvac:airFlow" => "airflow",
        "autodesk.spec.aec.hvac:pressure" | "autodesk.spec.aec.piping:pressure" => "pressure",
        "autodesk.spec.aec.hvac:coolingLoad"
        | "autodesk.spec.aec.hvac:heatingLoad"
        | "autodesk.spec.aec.hvac:heatGain"
        | "autodesk.spec.aec.hvac:power" => "thermal_load",
        "autodesk.spec.aec.hvac:velocity" | "autodesk.spec.aec.piping:velocity" => "velocity",
        "autodesk.spec.aec.hvac:temperature"
        | "autodesk.spec.aec.piping:temperature"
        | "autodesk.spec.aec.electrical:temperature" => "temperature",
        "autodesk.spec.aec:number" => "number",
        _ => return None,
    })
}
