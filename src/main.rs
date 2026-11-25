use anyhow::{Context, Result};
use clap::{ArgGroup, Parser};
use revit_view_extract::{
    mesh,
    model::RevitModel,
    output::{validate, write_json},
    room_federation::load_arch_rooms,
};
use std::path::PathBuf;

#[derive(Parser)]
#[command(version, about = "Extract engineering data from Revit 2024 locally")]
#[command(group(ArgGroup::new("architecture").args(["arch_rvt","arch_elements"])))]
struct Args {
    input: PathBuf,
    #[arg(long)]
    out: Option<PathBuf>,
    #[arg(long)]
    enums: Option<PathBuf>,
    #[arg(long, requires = "arch_rooms_csv")]
    arch_elements: Option<PathBuf>,
    #[arg(long, requires = "arch_elements")]
    arch_rooms_csv: Option<PathBuf>,
    #[arg(long, conflicts_with = "arch_elements")]
    arch_rvt: Option<PathBuf>,
    /// Architectural link instance ElementId when the model has multiple links.
    #[arg(long, requires = "architecture")]
    arch_link_id: Option<i64>,
    #[arg(long, alias = "mesh-gltf")]
    mesh: Option<PathBuf>,
    /// Small viewer descriptor JSON; the full model is written beside it.
    #[arg(long)]
    view_out: Option<PathBuf>,
    /// Full viewer model JSON path (default: <view-out stem>.model.json).
    #[arg(long, requires = "view_out")]
    view_model_out: Option<PathBuf>,
    #[arg(long)]
    parameters_out: Option<PathBuf>,
}

fn run() -> Result<()> {
    let args = Args::parse();
    let view_model_path = args
        .view_out
        .as_deref()
        .map(|descriptor| {
            revit_view_extract::view::model_path(descriptor, args.view_model_out.as_deref())
        })
        .transpose()?;
    let input_path = Some(args.input.clone());
    let inputs = [
        &input_path,
        &args.arch_rvt,
        &args.arch_elements,
        &args.arch_rooms_csv,
        &args.enums,
    ]
    .into_iter()
    .filter_map(|p| p.as_deref())
    .collect::<Vec<_>>();
    let outputs = [
        &args.out,
        &args.mesh,
        &args.view_out,
        &view_model_path,
        &args.parameters_out,
    ]
    .into_iter()
    .filter_map(|p| p.as_deref())
    .collect::<Vec<_>>();
    validate(&inputs, &outputs)?;
    let print_json = outputs.is_empty();
    let mut model = RevitModel::from_rvt(&args.input)?;
    if let Some(path) = args.enums {
        model.set_enums(serde_json::from_slice(&std::fs::read(path)?)?)?;
    }
    let rooms = if let Some(path) = args.arch_rvt {
        Some(mesh::architectural_rooms(&path)?)
    } else if let (Some(elements), Some(csv)) = (args.arch_elements, args.arch_rooms_csv) {
        Some(load_arch_rooms(&elements, &csv)?)
    } else {
        None
    };
    let mut result = model.extract_all(rooms.as_deref(), args.arch_link_id)?;
    if let Some(path) = args.parameters_out {
        let mut parameters = serde_json::to_value(&model.parameters)?;
        revit_view_extract::wire::encode_element_ids(&mut parameters)?;
        write_json(&path, &parameters)?;
    }
    let scene = if args.mesh.is_some() || args.view_out.is_some() {
        Some(mesh::build(&args.input)?)
    } else {
        None
    };
    if let (Some(scene), Some(path)) = (&scene, &args.mesh) {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        scene.write(path)?;
        result["mesh"] = serde_json::to_value(&scene.metrics)?;
    }
    if let Some(path) = args.view_out {
        let mut view = serde_json::json!({"schema_version":1,"source":{"name":model.name},"engineering":result,
            "mesh":scene.map(|scene|revit_view_extract::view::mesh_payload(scene.gltf,serde_json::to_value(scene.metrics)?)).transpose()?});
        revit_view_extract::view::write_files(
            &path,
            view_model_path
                .as_deref()
                .context("Viewer model output was not derived")?,
            &mut view,
        )?;
    }
    if let Some(path) = args.out {
        write_json(&path, &result)?;
        eprintln!(
            "Extracted {}% of {} declared elements",
            result["elem_table_coverage_pct"], result["elem_table_total"]
        );
    } else if print_json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        eprintln!(
            "Extracted {}% of {} declared elements",
            result["elem_table_coverage_pct"], result["elem_table_total"]
        );
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error:#}");
        std::process::exit(2);
    }
}
