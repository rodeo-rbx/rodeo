use anyhow::{bail, Context, Result};
use rodeo_client::RodeoClient;

fn short(id: &str) -> &str {
    &id[..8.min(id.len())]
}

/// `rodeo save [studio-id] [--out path]`.
///
/// Resolves the target Studio from state (prefix accepted; defaults to the
/// only connected one), triggers the mtime-verified AX save on its working
/// file, then commits the result: to `--out` when given, otherwise back to
/// the launch's SOURCE_PATH (the file the user actually asked to open — the
/// working file is a temp copy for default launches). No destination (blank
/// place, no --out) just reports the saved working file.
pub async fn main(id: Option<&str>, host: &str, port: u16, out: Option<String>) -> Result<()> {
    if matches!(id, Some("")) {
        bail!("provide a Studio ID or omit it to target the only connected Studio");
    }
    let client = RodeoClient::connect(host, port)?;
    client.check_version().await?;
    let snapshot = client.get_state().await?;

    let matches: Vec<_> = snapshot
        .studios
        .iter()
        .filter(|st| id.is_none_or(|p| st.studio_id.starts_with(p)))
        .collect();
    let studio = match (id, matches.as_slice()) {
        (Some(id), []) => bail!("no Studio with id '{id}' (see `rodeo state`)"),
        (None, []) => bail!("no connected Studio"),
        (_, [one]) => *one,
        (_, many) => bail!(
            "{} — specify one of: {}",
            if id.is_some() { "ambiguous Studio id" } else { "multiple Studios connected" },
            many.iter()
                .map(|st| short(&st.studio_id).to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    };
    save_studio(&client, studio, out).await
}

/// The launch session rodeo saves `studio` through. Only Studios rodeo
/// launched have one.
pub(crate) fn save_session(studio: &rodeo_proto::StudioState) -> Result<String> {
    match studio.session_id.clone() {
        Some(session_guid) => Ok(session_guid),
        None => bail!(
            "Studio {} was not launched by rodeo (no session) — save it from Studio directly",
            short(&studio.studio_id)
        ),
    }
}

/// Save `studio` and commit the saved file to `out`, else to the launch's
/// SOURCE_PATH. Shared by `rodeo save` and `rodeo run --studio-id … --save`.
pub(crate) async fn save_studio(client: &RodeoClient, studio: &rodeo_proto::StudioState, out: Option<String>) -> Result<()> {
    let studio_short = short(&studio.studio_id);
    let session_guid = save_session(studio)?;

    // Backend fires the AX save and confirms via working-file mtime change
    // (retrying up to 60s); a failure here is a hard error, never a silent
    // exit 0.
    let result = client.save_place(Some(session_guid)).await?;
    let Some(working) = result.path else {
        tracing::info!("Saved Studio {studio_short}");
        return Ok(());
    };

    // Commit destination: --out wins, else the launch's source file. Skipped
    // when the save already landed in the destination (SaveInPlace) or there
    // is no source (blank place).
    let dest = out.or_else(|| studio.source_path.clone()).filter(|d| *d != working);
    let Some(dest) = dest else {
        tracing::info!("Saved Studio {studio_short} ({working})");
        return Ok(());
    };

    // Brief settle for Studio to finish flushing the file after the mtime
    // first moved, then copy atomically so a failed copy can't leave a
    // truncated destination.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    if let Some(parent) = std::path::Path::new(&dest).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .context(format!("failed to create parent dirs for {dest}"))?;
        }
    }
    let tmp = format!("{dest}.tmp");
    std::fs::copy(&working, &tmp).context(format!("failed to copy {working} to {tmp}"))?;
    std::fs::rename(&tmp, &dest).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        anyhow::anyhow!("failed to move {tmp} to {dest}: {e}")
    })?;
    tracing::info!("Saved Studio {studio_short} -> {dest}");

    Ok(())
}
