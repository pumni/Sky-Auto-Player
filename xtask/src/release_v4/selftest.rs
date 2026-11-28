use super::*;

pub(super) fn self_test() -> Result<()> {
    let outcomes = [
        mock_pre_token_latest_guard_stable_and_beta(),
        mock_pre_token_latest_guard_rejects_invalid_state_and_identity(),
        mock_pre_token_read_only_transport_rejects_mutations(),
        mock_pre_token_latest_guard_job_authorization(),
        mock_publish_succeeds(),
        mock_draft_readback_failures_never_patch(),
        mock_non_monotonic_candidate_never_mutates(),
        mock_draft_post_timeout_reconciles_exact_marker(),
        mock_upload_timeout_reconciles_exact_asset(),
        mock_complete_transaction_and_raw_metadata(),
        mock_metadata_promotion_is_idempotent_after_committed_timeout(),
        mock_metadata_promotion_rejects_changed_latest(),
        mock_publication_identity_and_asset_inventory_fail_closed(),
        mock_pr_merge_sha_does_not_break_release_or_owned_cleanup(),
        mock_conflicting_draft_is_not_deleted(),
        mock_digest_mismatch_cleans_owned_draft(),
        mock_partial_upload_cleans_owned_draft(),
        mock_cleanup_failure_preserves_original_error(),
        mock_patch_timeout_reconciles_exact_release_id(),
        mock_unknown_publication_never_cleans_up(),
        mock_latest_policy_failure_is_post_publication(),
        metadata_token_probe_rejects_wrong_repository_state(),
        mock_stale_evidence_identity_fails_closed(),
        mock_manifest_path_traversal_fails_closed(),
        mock_release_doctor_detects_metadata_endpoint_divergence(),
        doctor_state_input_rejects_malformed_and_oversized_files(),
        retry_policy_retries_only_transient_responses(),
        marker_parser_rejects_duplicates_and_multiple_markers(),
        mutation_adapter_is_at_most_once(),
        raw_metadata_retry_converges_after_cache_lag(),
        raw_metadata_retry_fails_with_bounded_diagnostics(),
        runtime_identity_fails_closed_without_exact_workflow_context(),
        workflow_run_identity_fails_closed(),
        timestamp_normalization_is_utc_and_strict(),
    ];
    let scenario_count = outcomes.len();
    for outcome in outcomes {
        outcome?;
    }
    println!(
        "V4 Rust release state-machine mock self-test: PASS ({} scenarios)",
        scenario_count
    );
    Ok(())
}

fn mock_pre_token_latest_guard_stable_and_beta() -> Result<()> {
    let (_root, options, base_context, _) = fixture()?;
    let stable_context = latest_guard_context(&base_context, "99.0.0", "stable");
    let stable_state = latest_guard_published_state(&stable_context, 12);
    let stable_release = fake_release(
        12,
        &stable_context.tag,
        &stable_context.source_sha,
        false,
        false,
        true,
    );
    let mut stable_github = FakeTransport::new();
    stable_github.latest = stable_release.clone();
    stable_github.releases.insert(12, stable_release);
    verify_latest_before_metadata_token_with_state(
        &options,
        &stable_context,
        &stable_state,
        &mut stable_github,
    )?;
    assert_eq!(
        stable_github.events,
        [
            "GET repos/pumni/Sky-Auto-Player/releases/12",
            "GET repos/pumni/Sky-Auto-Player/releases/latest"
        ]
    );

    let beta_context = latest_guard_context(&base_context, "99.0.0-beta.1", "beta");
    let beta_state = latest_guard_published_state(&beta_context, 12);
    let beta_release = fake_release(
        12,
        &beta_context.tag,
        &beta_context.source_sha,
        false,
        true,
        true,
    );
    let stable_latest = fake_release(11, "v98.0.0", "b".repeat(40), false, false, true);
    write_json_atomic(
        &options.state_root.join("preflight-evidence.json"),
        &json!({"pre_publication_latest":stable_latest}),
    )?;
    let mut beta_github = FakeTransport::new();
    beta_github.latest = stable_latest;
    beta_github.releases.insert(12, beta_release);
    verify_latest_before_metadata_token_with_state(
        &options,
        &beta_context,
        &beta_state,
        &mut beta_github,
    )?;
    assert_eq!(
        beta_github.events,
        [
            "GET repos/pumni/Sky-Auto-Player/releases/12",
            "GET repos/pumni/Sky-Auto-Player/releases/latest"
        ]
    );
    Ok(())
}

fn mock_pre_token_latest_guard_rejects_invalid_state_and_identity() -> Result<()> {
    let (_root, options, base_context, _) = fixture()?;
    let stable_context = latest_guard_context(&base_context, "99.0.0", "stable");
    let state = latest_guard_published_state(&stable_context, 12);
    let latest = fake_release(
        12,
        &stable_context.tag,
        &stable_context.source_sha,
        false,
        false,
        true,
    );
    let wrong_source = if stable_context.source_sha == "a".repeat(40) {
        "b".repeat(40)
    } else {
        "a".repeat(40)
    };
    for (label, published) in [
        (
            "replayed tag",
            fake_release(
                12,
                "v98.0.0",
                &stable_context.source_sha,
                false,
                false,
                true,
            ),
        ),
        (
            "replayed source SHA",
            fake_release(12, &stable_context.tag, wrong_source, false, false, true),
        ),
        (
            "wrong release ID",
            fake_release(
                13,
                &stable_context.tag,
                &stable_context.source_sha,
                false,
                false,
                true,
            ),
        ),
    ] {
        let mut github = FakeTransport::new();
        github.latest = latest.clone();
        github.releases.insert(12, published);
        if verify_latest_before_metadata_token_with_state(
            &options,
            &stable_context,
            &state,
            &mut github,
        )
        .is_ok()
        {
            return Err(format!("pre-token guard accepted {label}").into());
        }
        if github.events.iter().any(|event| !event.starts_with("GET ")) {
            return Err(format!("pre-token guard mutated GitHub after {label}").into());
        }
    }

    let mut missing_state = fixture_state(&stable_context);
    missing_state.release_id = None;
    let mut missing_github = FakeTransport::new();
    if verify_latest_before_metadata_token_with_state(
        &options,
        &stable_context,
        &missing_state,
        &mut missing_github,
    )
    .is_ok()
        || !missing_github.events.is_empty()
    {
        return Err(
            "pre-token guard did not reject missing published state before GitHub access".into(),
        );
    }

    let mut changed_stable = FakeTransport::new();
    changed_stable.releases.insert(12, latest.clone());
    changed_stable.latest = fake_release(11, "v98.0.0", "b".repeat(40), false, false, true);
    if verify_latest_before_metadata_token_with_state(
        &options,
        &stable_context,
        &state,
        &mut changed_stable,
    )
    .is_ok()
    {
        return Err("pre-token guard accepted changed stable Latest".into());
    }
    if changed_stable
        .events
        .iter()
        .any(|event| !event.starts_with("GET "))
    {
        return Err("pre-token guard mutated GitHub after changed stable Latest".into());
    }

    let beta_context = latest_guard_context(&base_context, "99.0.0-beta.1", "beta");
    let beta_state = latest_guard_published_state(&beta_context, 12);
    let beta_release = fake_release(
        12,
        &beta_context.tag,
        &beta_context.source_sha,
        false,
        true,
        true,
    );
    let preflight_latest = fake_release(11, "v98.0.0", "b".repeat(40), false, false, true);
    write_json_atomic(
        &options.state_root.join("preflight-evidence.json"),
        &json!({"pre_publication_latest":preflight_latest}),
    )?;
    let mut changed_beta = FakeTransport::new();
    changed_beta.releases.insert(12, beta_release);
    changed_beta.latest = fake_release(10, "v97.0.0", "c".repeat(40), false, false, true);
    if verify_latest_before_metadata_token_with_state(
        &options,
        &beta_context,
        &beta_state,
        &mut changed_beta,
    )
    .is_ok()
    {
        return Err("pre-token guard accepted changed beta Latest".into());
    }
    if changed_beta
        .events
        .iter()
        .any(|event| !event.starts_with("GET "))
    {
        return Err("pre-token guard mutated GitHub after changed beta Latest".into());
    }
    Ok(())
}

fn mock_pre_token_read_only_transport_rejects_mutations() -> Result<()> {
    let mut github = FakeTransport::new();
    {
        let mut read_only = ReadOnlyTransport { inner: &mut github };
        let body = json!({"mutation":true});
        for method in ["POST", "PATCH", "PUT", "DELETE"] {
            if read_only
                .request(
                    method,
                    "repos/pumni/Sky-Auto-Player/releases/12",
                    Some(&body),
                )
                .is_ok()
            {
                return Err(format!("read-only transport accepted {method}").into());
            }
        }
        if read_only
            .request(
                "GET",
                "repos/pumni/Sky-Auto-Player/releases/latest",
                Some(&body),
            )
            .is_ok()
            || read_only
                .upload(
                    "uploads/release/12",
                    "installer.exe",
                    Path::new("installer.exe"),
                )
                .is_ok()
            || read_only
                .download(
                    "repos/pumni/Sky-Auto-Player/releases/12/assets/1",
                    Path::new("installer.exe"),
                )
                .is_ok()
            || read_only
                .raw_get("https://raw.githubusercontent.com/example/latest.json")
                .is_ok()
        {
            return Err(
                "read-only transport accepted a body, upload, download, or raw request".into(),
            );
        }
    }
    if !github.events.is_empty() {
        return Err("read-only transport forwarded a rejected operation".into());
    }
    Ok(())
}

fn mock_pre_token_latest_guard_job_authorization() -> Result<()> {
    let command = "verify-latest-before-metadata-token";
    if expected_job_for_command(command) != Some("metadata_promote")
        || validate_dispatch_job(command, "metadata_promote")? != "metadata_promote"
    {
        return Err("pre-token Latest guard is not bound to metadata_promote".into());
    }
    for other_job in ["metadata_probe", "publish", "final_verify", "prepare"] {
        if validate_dispatch_job(command, other_job).is_ok() {
            return Err(
                format!("pre-token Latest guard accepted noncanonical job {other_job}").into(),
            );
        }
    }
    Ok(())
}

fn latest_guard_context(base: &Context, version: &str, channel: &str) -> Context {
    let mut context = base.clone();
    context.version = version.to_owned();
    context.channel = channel.to_owned();
    context.tag = format!("v{version}");
    context.release_notes_path = format!("docs/releases/{}.md", context.tag);
    context
}

fn latest_guard_published_state(context: &Context, release_id: u64) -> ReleaseState {
    let mut state = fixture_state(context);
    state.phase = Phase::PublishedPendingMetadata;
    state.release_id = Some(release_id);
    state.published_at = Some("2026-01-01T00:00:00Z".to_owned());
    state
}

pub(super) fn mock_publish_succeeds() -> Result<()> {
    let (root, options, context, manifest) = fixture()?;
    let mut github = FakeTransport::new();
    let old_latest = fake_release(11, "v4.1.4", "b".repeat(40), false, false, true);
    github.latest = old_latest.clone();
    write_json_atomic(
        &options.state_root.join("preflight-evidence.json"),
        &json!({"pre_publication_latest":old_latest}),
    )?;
    write_json_atomic(&options.state_root.join("release-context.json"), &context)?;
    write_json_atomic(
        &options.state_root.join("release-state.json"),
        &fixture_state(&context),
    )?;
    write_json_atomic(
        &options.state_root.join("candidate-manifest.json"),
        &manifest,
    )?;
    publish_release_with_mock_verifier(&root, &options, &mut github)?;
    let state: ReleaseState = read_json(&options.state_root.join("release-state.json"))?;
    assert_eq!(state.phase, Phase::PublishedPendingMetadata);
    assert_eq!(state.release_id, Some(12));
    assert!(state.qualified_after_download);
    let post = github
        .events
        .iter()
        .position(|event| event == "PATCH repos/pumni/Sky-Auto-Player/releases/12")
        .ok_or("publish PATCH missing")?;
    let downloads = github
        .events
        .iter()
        .enumerate()
        .filter(|(_, event)| event.starts_with("DOWNLOAD "))
        .collect::<Vec<_>>();
    assert_eq!(
        downloads.len(),
        2,
        "draft installer and signature are read back"
    );
    assert!(downloads.iter().all(|(index, _)| *index < post));
    let verify = github
        .events
        .iter()
        .rposition(|event| event == "GET repos/pumni/Sky-Auto-Player/releases/12")
        .ok_or("exact id reconciliation missing")?;
    assert!(
        post < verify,
        "exact release id must be read after irreversible publication"
    );
    assert!(github.deletions.is_empty());
    let _ = fs::remove_dir_all(&options.state_root);
    Ok(())
}

pub(super) fn mock_draft_readback_failures_never_patch() -> Result<()> {
    for failure in 0..4 {
        let (root, options, context, manifest) = fixture()?;
        let mut github = FakeTransport::new();
        github.latest = fake_release(11, "v4.1.4", "b".repeat(40), false, false, true);
        prepare_publish_fixture(&options, &context, &manifest, &github.latest)?;
        let result = match failure {
            0 => {
                github.fail_download = true;
                publish_release_with_mock_verifier(&root, &options, &mut github)
            }
            1 => {
                github.corrupt_download = true;
                publish_release_with_mock_verifier(&root, &options, &mut github)
            }
            2 => {
                github.corrupt_signature_download = true;
                publish_release_with_mock_verifier(&root, &options, &mut github)
            }
            _ => publish_release_with_signature_verifier(
                &root,
                &options,
                &mut github,
                &crate::updater_trust::canonical_key_id()?,
                |installer, signature, version| {
                    let expected_installer = release_metadata::canonical_installer_name(version);
                    let expected_signature = format!("{expected_installer}.sig");
                    if installer.file_name().and_then(|name| name.to_str())
                        != Some(expected_installer.as_str())
                        || signature.file_name().and_then(|name| name.to_str())
                            != Some(expected_signature.as_str())
                    {
                        return Err(
                            "downloaded verifier input filenames were not source names".into()
                        );
                    }
                    Err("injected cryptographic signature verification failure".into())
                },
            ),
        };
        let error = result.expect_err("failed draft readback must block publication");
        match failure {
            0 => assert!(
                error
                    .to_string()
                    .contains("injected draft asset download failure")
            ),
            1 => assert!(error.to_string().contains("draft asset readback differs")),
            2 => {
                assert!(!github.corrupt_signature_download);
                assert!(error.to_string().contains("draft asset readback differs"));
            }
            _ => assert!(
                error
                    .to_string()
                    .contains("injected cryptographic signature")
            ),
        }
        assert!(
            github
                .events
                .iter()
                .all(|event| event != "PATCH repos/pumni/Sky-Auto-Player/releases/12")
        );
        let state: ReleaseState = read_json(&options.state_root.join("release-state.json"))?;
        assert!(!state.qualified_after_download);
        assert_eq!(github.deletions, [12]);
        let _ = fs::remove_dir_all(&options.state_root);
    }
    Ok(())
}

fn mock_non_monotonic_candidate_never_mutates() -> Result<()> {
    {
        let (root, options, context, manifest) = fixture()?;
        let mut github = FakeTransport::new();
        github.latest = fake_release(11, "v4.1.4", "b".repeat(40), false, false, true);
        let signature = manifest
            .qualification_assets
            .iter()
            .find(|record| record.role == "updater-signature")
            .ok_or("fixture signature missing")?;
        let current_path = options.state_root.join("same-version-latest.json");
        release_metadata::generate(release_metadata::GenerateInput {
            channel: release_metadata::Channel::parse(&context.channel)?,
            version: &context.version,
            notes_path: &root.join(&context.release_notes_path),
            pub_date: "2026-01-01T00:00:00Z",
            platform: release_metadata::WINDOWS_PLATFORM,
            asset_url: &release_metadata::canonical_asset_url(&context.version),
            signature_path: &safe_state_asset_path(&options.state_root, signature)?,
            output: &current_path,
        })?;
        github
            .metadata_files
            .insert(context.channel.clone(), fs::read(&current_path)?);
        prepare_publish_fixture(&options, &context, &manifest, &github.latest)?;
        assert!(publish_release_with_mock_verifier(&root, &options, &mut github).is_err());
        assert!(github.events.iter().all(|event| {
            !(event.starts_with("POST ")
                || event.starts_with("PATCH ")
                || event.starts_with("PUT ")
                || event.starts_with("DELETE ")
                || event.starts_with("UPLOAD "))
        }));
        let _ = fs::remove_dir_all(&options.state_root);
    }
    {
        let (root, options, context, manifest) = fixture()?;
        let mut github = FakeTransport::new();
        github.latest = fake_release(11, &context.tag, &context.source_sha, false, false, true);
        prepare_publish_fixture(&options, &context, &manifest, &github.latest)?;
        assert!(publish_release_with_mock_verifier(&root, &options, &mut github).is_err());
        assert!(github.events.iter().all(|event| {
            !(event.starts_with("POST ")
                || event.starts_with("PATCH ")
                || event.starts_with("PUT ")
                || event.starts_with("DELETE ")
                || event.starts_with("UPLOAD "))
        }));
        let _ = fs::remove_dir_all(&options.state_root);
    }
    Ok(())
}

pub(super) fn mock_draft_post_timeout_reconciles_exact_marker() -> Result<()> {
    let (root, options, context, manifest) = fixture()?;
    let mut github = FakeTransport::new();
    github.latest = fake_release(11, "v4.1.4", "b".repeat(40), false, false, true);
    github.post_created_then_timeout = true;
    prepare_publish_fixture(&options, &context, &manifest, &github.latest)?;
    publish_release_with_mock_verifier(&root, &options, &mut github)?;
    let state: ReleaseState = read_json(&options.state_root.join("release-state.json"))?;
    assert_eq!(state.phase, Phase::PublishedPendingMetadata);
    assert_eq!(state.release_id, Some(12));
    assert_eq!(
        github.releases.len(),
        1,
        "timeout reconciliation must not duplicate the draft"
    );
    assert!(github.deletions.is_empty());
    let _ = fs::remove_dir_all(&options.state_root);
    Ok(())
}

fn mock_upload_timeout_reconciles_exact_asset() -> Result<()> {
    let (root, options, context, manifest) = fixture()?;
    let mut github = FakeTransport::new();
    github.latest = fake_release(11, "v4.1.4", "b".repeat(40), false, false, true);
    github.fail_upload_after_commit = true;
    prepare_publish_fixture(&options, &context, &manifest, &github.latest)?;
    publish_release_with_mock_verifier(&root, &options, &mut github)?;
    assert_eq!(
        github
            .events
            .iter()
            .filter(|event| event.starts_with("UPLOAD "))
            .count(),
        manifest.public_assets.len(),
        "committed upload timeout must be read back rather than blindly replayed"
    );
    let first_upload = github
        .events
        .iter()
        .position(|event| event.starts_with("UPLOAD "))
        .ok_or("expected first upload event")?;
    let reconcile = github
        .events
        .iter()
        .position(|event| event == "GET repos/pumni/Sky-Auto-Player/releases/12")
        .ok_or("exact release readback after upload timeout missing")?;
    let second_upload = github
        .events
        .iter()
        .enumerate()
        .skip(first_upload + 1)
        .find(|(_, event)| event.starts_with("UPLOAD "))
        .map(|(index, _)| index)
        .ok_or("expected second upload event")?;
    assert!(first_upload < reconcile && reconcile < second_upload);
    let _ = fs::remove_dir_all(&options.state_root);
    Ok(())
}

pub(super) fn mock_complete_transaction_and_raw_metadata() -> Result<()> {
    let (root, options, context, manifest) = fixture()?;
    let mut github = FakeTransport::new();
    github.latest = fake_release(11, "v4.1.4", "b".repeat(40), false, false, true);
    prepare_publish_fixture(&options, &context, &manifest, &github.latest)?;
    publish_release_with_mock_verifier(&root, &options, &mut github)?;
    promote_metadata(&root, &options, &mut github)?;
    final_verify(&root, &options, &mut github)?;
    let state: ReleaseState = read_json(&options.state_root.join("release-state.json"))?;
    assert_eq!(state.phase, Phase::Complete);
    assert!(state.final_verified);
    assert!(github.metadata.starts_with(b"{\n"));
    assert!(
        github
            .events
            .iter()
            .any(|event| event.starts_with("PUT repos/pumni/Sky-Auto-Player/contents/channels/"))
    );
    let run_id = "9001";
    github.workflow_runs.insert(
        run_id.to_owned(),
        json!({"id":9001,"path":".github/workflows/release-v4.yml","event":"workflow_dispatch",
            "repository":REPOSITORY,"head_sha":context.source_sha,"status":"completed","conclusion":"success",
            "html_url":"https://github.com/pumni/Sky-Auto-Player/actions/runs/9001"}),
    );
    let event_count = github.events.len();
    let metadata_before = github.metadata_files.get("stable").cloned();
    let (report, _) = doctor_report(
        &root,
        &[
            "--tag".to_owned(),
            context.tag.clone(),
            "--version".to_owned(),
            context.version.clone(),
            "--channel".to_owned(),
            context.channel.clone(),
            "--workflow-sha".to_owned(),
            context.source_sha.clone(),
            "--run-id".to_owned(),
            run_id.to_owned(),
            "--state-path".to_owned(),
            options
                .state_root
                .join("release-state.json")
                .to_string_lossy()
                .into_owned(),
        ],
        &mut github,
    )?;
    assert_eq!(report["external_phase"], "COMPLETE");
    assert!(report["classification"].is_null());
    assert_eq!(report["external_truth"]["metadata_bytes_match"], true);
    assert_eq!(
        github.metadata_files.get("stable"),
        metadata_before.as_ref()
    );
    assert!(github.deletions.is_empty());
    assert!(
        github.events[event_count..]
            .iter()
            .all(|event| event.starts_with("GET ") || event.starts_with("RAW_GET "))
    );
    let _ = fs::remove_dir_all(&options.state_root);
    Ok(())
}

pub(super) fn mock_metadata_promotion_is_idempotent_after_committed_timeout() -> Result<()> {
    let (root, options, context, manifest) = fixture()?;
    let mut github = FakeTransport::new();
    github.latest = fake_release(11, "v4.1.4", "b".repeat(40), false, false, true);
    prepare_publish_fixture(&options, &context, &manifest, &github.latest)?;
    publish_release_with_mock_verifier(&root, &options, &mut github)?;
    github.fail_metadata_put_after_write = true;
    promote_metadata(&root, &options, &mut github)?;
    assert!(github.metadata_files.contains_key("stable"));
    assert_eq!(
        github
            .events
            .iter()
            .filter(|event| event.starts_with("PUT repos/pumni/Sky-Auto-Player/contents/channels/"))
            .count(),
        1
    );

    promote_metadata(&root, &options, &mut github)?;
    assert_eq!(
        github
            .events
            .iter()
            .filter(|event| event.starts_with("PUT repos/pumni/Sky-Auto-Player/contents/channels/"))
            .count(),
        1,
        "retry must reconcile the exact bytes without a second metadata write"
    );
    let state: ReleaseState = read_json(&options.state_root.join("release-state.json"))?;
    assert!(state.promoted_at.is_some());
    assert!(github.deletions.is_empty());
    let _ = fs::remove_dir_all(&options.state_root);
    Ok(())
}

pub(super) fn mock_metadata_promotion_rejects_changed_latest() -> Result<()> {
    let (root, options, context, manifest) = fixture()?;
    let mut github = FakeTransport::new();
    github.latest = fake_release(11, "v4.1.4", "b".repeat(40), false, false, true);
    prepare_publish_fixture(&options, &context, &manifest, &github.latest)?;
    publish_release_with_mock_verifier(&root, &options, &mut github)?;
    github.latest = fake_release(13, "v4.4.0", "c".repeat(40), false, false, true);

    let error = promote_metadata(&root, &options, &mut github)
        .expect_err("changed GitHub Latest must block metadata promotion");
    assert!(
        error
            .to_string()
            .contains("GitHub Latest changed after publication")
    );
    assert!(github.events.iter().all(|event| !event.starts_with("PUT ")));
    let _ = fs::remove_dir_all(&options.state_root);
    Ok(())
}

pub(super) fn mock_publication_identity_and_asset_inventory_fail_closed() -> Result<()> {
    let (root, options, context, manifest) = fixture()?;
    let mut github = FakeTransport::new();
    github.latest = fake_release(11, "v4.1.4", "b".repeat(40), false, false, true);
    prepare_publish_fixture(&options, &context, &manifest, &github.latest)?;
    publish_release_with_mock_verifier(&root, &options, &mut github)?;
    let published = github
        .releases
        .get(&12)
        .cloned()
        .ok_or("fake published release missing")?;
    verify_public_assets(&mut github, &published, &manifest.public_assets, false)?;
    validate_published_release(&published, &context, 12)?;

    let mut missing_asset = published.clone();
    missing_asset["assets"].as_array_mut().unwrap().pop();
    assert!(
        verify_public_assets(&mut github, &missing_asset, &manifest.public_assets, false).is_err()
    );

    let mut extra_asset = published.clone();
    extra_asset["assets"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name":"unexpected.bin"}));
    assert!(
        verify_public_assets(&mut github, &extra_asset, &manifest.public_assets, false).is_err()
    );

    let mut wrong_digest = published.clone();
    wrong_digest["assets"][0]["digest"] = json!("sha256:invalid");
    assert!(
        verify_public_assets(&mut github, &wrong_digest, &manifest.public_assets, false).is_err()
    );

    let mut wrong_tag = published.clone();
    wrong_tag["tag_name"] = json!("v4.99.0");
    assert!(validate_published_release(&wrong_tag, &context, 12).is_err());
    let mut wrong_source = published.clone();
    wrong_source["target_commitish"] = json!("c".repeat(40));
    assert!(validate_published_release(&wrong_source, &context, 12).is_err());
    let mut wrong_id = published.clone();
    wrong_id["id"] = json!(13);
    assert!(validate_published_release(&wrong_id, &context, 12).is_err());
    let mut mixed_state = published.clone();
    mixed_state["draft"] = json!(true);
    assert!(validate_published_release(&mixed_state, &context, 12).is_err());
    let mut wrong_channel = published.clone();
    wrong_channel["prerelease"] = json!(true);
    assert!(validate_published_release(&wrong_channel, &context, 12).is_err());
    let mut mutable_release = published;
    mutable_release["immutable"] = json!(false);
    assert!(validate_published_release(&mutable_release, &context, 12).is_err());

    assert!(github.deletions.is_empty());
    let _ = fs::remove_dir_all(&options.state_root);
    Ok(())
}

pub(super) fn mock_release_doctor_detects_metadata_endpoint_divergence() -> Result<()> {
    let (root, options, context, manifest) = fixture()?;
    let mut github = FakeTransport::new();
    github.latest = fake_release(11, "v4.1.4", "b".repeat(40), false, false, true);
    prepare_publish_fixture(&options, &context, &manifest, &github.latest)?;
    publish_release_with_mock_verifier(&root, &options, &mut github)?;
    promote_metadata(&root, &options, &mut github)?;
    final_verify(&root, &options, &mut github)?;
    github.workflow_runs.insert(
        "9001".to_owned(),
        json!({"id":9001,"path":".github/workflows/release-v4.yml","event":"workflow_dispatch",
            "repository":REPOSITORY,"head_sha":context.source_sha,"status":"completed","conclusion":"success"}),
    );
    let mut divergent = github
        .raw_metadata_files
        .get("stable")
        .cloned()
        .ok_or("fake raw metadata was not promoted")?;
    divergent.push(b' ');
    github
        .raw_metadata_files
        .insert("stable".to_owned(), divergent);
    let (report, _) = doctor_report(
        &root,
        &[
            "--tag".to_owned(),
            context.tag.clone(),
            "--version".to_owned(),
            context.version.clone(),
            "--channel".to_owned(),
            context.channel.clone(),
            "--workflow-sha".to_owned(),
            context.source_sha.clone(),
            "--run-id".to_owned(),
            "9001".to_owned(),
        ],
        &mut github,
    )?;
    assert_eq!(report["classification"], "POST_PUBLICATION_INCIDENT");
    assert_eq!(report["external_truth"]["metadata_bytes_match"], false);
    assert!(report["findings"].as_array().is_some_and(|items| {
        items
            .iter()
            .any(|item| item == "Contents API metadata differs from the public raw endpoint")
    }));
    assert!(github.deletions.is_empty());
    let _ = fs::remove_dir_all(&options.state_root);
    Ok(())
}

pub(super) fn doctor_state_input_rejects_malformed_and_oversized_files() -> Result<()> {
    let (_root, options, context, _) = fixture()?;
    let state_path = options.state_root.join("doctor-input.json");
    let args = vec![
        "--tag".to_owned(),
        context.tag.clone(),
        "--version".to_owned(),
        context.version.clone(),
        "--channel".to_owned(),
        context.channel.clone(),
        "--state-path".to_owned(),
        state_path.to_string_lossy().into_owned(),
    ];
    let mut github = FakeTransport::new();
    fs::write(&state_path, b"{ malformed")?;
    assert!(doctor_report(&repo::root(), &args, &mut github).is_err());
    assert!(
        github.events.is_empty(),
        "malformed local state must fail before remote reads"
    );

    fs::write(&state_path, vec![b' '; 16 * 1024 * 1024 + 1])?;
    assert!(doctor_report(&repo::root(), &args, &mut github).is_err());
    assert!(
        github.events.is_empty(),
        "oversized local state must fail before remote reads"
    );

    fs::write(&state_path, serde_json::to_vec(&json!({"tag":"v4.0.0"}))?)?;
    assert!(doctor_report(&repo::root(), &args, &mut github).is_err());
    assert!(
        github.events.is_empty(),
        "divergent local identity must fail before remote reads"
    );
    let _ = fs::remove_dir_all(&options.state_root);
    Ok(())
}

pub(super) fn retry_policy_retries_only_transient_responses() -> Result<()> {
    let mut attempts = 0;
    let mut delays = Vec::new();
    let result = retry_github(
        || {
            attempts += 1;
            if attempts < 3 {
                Err(Box::new(HttpFailure {
                    status: Some(if attempts == 1 { 429 } else { 503 }),
                    detail: "injected transient response".to_owned(),
                })
                    as Box<dyn std::error::Error + Send + Sync>)
            } else {
                Ok("ok")
            }
        },
        |delay| delays.push(delay),
    )?;
    assert_eq!(result, "ok");
    assert_eq!(attempts, 3);
    assert_eq!(delays, [Duration::from_millis(500), Duration::from_secs(1)]);

    for status in [403, 404] {
        let mut attempts = 0;
        let error = retry_github::<()>(
            || {
                attempts += 1;
                Err(Box::new(HttpFailure {
                    status: Some(status),
                    detail: "injected permanent response".to_owned(),
                })
                    as Box<dyn std::error::Error + Send + Sync>)
            },
            |_| panic!("permanent HTTP failures must not sleep"),
        )
        .expect_err("403 and 404 must fail without retry");
        assert_eq!(
            error
                .downcast_ref::<HttpFailure>()
                .and_then(|failure| failure.status),
            Some(status)
        );
        assert_eq!(attempts, 1);
    }

    let mut attempts = 0;
    let error = retry_github::<()>(
        || {
            attempts += 1;
            Err(Box::new(HttpFailure {
                status: Some(500),
                detail: "injected persistent server error".to_owned(),
            }) as Box<dyn std::error::Error + Send + Sync>)
        },
        |_| {},
    )
    .expect_err("retryable server failures must eventually exhaust");
    assert_eq!(
        error
            .downcast_ref::<HttpFailure>()
            .and_then(|failure| failure.status),
        Some(500)
    );
    assert_eq!(attempts, GITHUB_RETRY_ATTEMPTS);
    Ok(())
}

fn mutation_adapter_is_at_most_once() -> Result<()> {
    for method in ["POST", "PATCH", "PUT", "DELETE"] {
        let attempts = std::cell::Cell::new(0);
        let error = request_with_retry_policy(
            method,
            "repos/pumni/Sky-Auto-Player/releases/12",
            || {
                attempts.set(attempts.get() + 1);
                Err::<(), _>(Box::new(HttpFailure {
                    status: Some(504),
                    detail: "injected committed mutation timeout".to_owned(),
                })
                    as Box<dyn std::error::Error + Send + Sync>)
            },
            |_| panic!("mutations must never enter the retry sleep path"),
        )
        .expect_err("mutation failures must be marked for reconciliation");
        assert_eq!(attempts.get(), 1, "{method} was replayed");
        let unknown = error
            .downcast_ref::<MutationOutcomeUnknown>()
            .ok_or("mutation error did not carry the typed uncertain outcome")?;
        assert_eq!(unknown.method, method);
    }
    Ok(())
}

fn raw_metadata_retry_converges_after_cache_lag() -> Result<()> {
    let expected = b"exact-qualified-metadata".to_vec();
    let mut responses = std::collections::VecDeque::from([
        Ok(None),
        Ok(Some(b"stale-cdn-bytes".to_vec())),
        Ok(Some(expected.clone())),
    ]);
    let reads = std::cell::Cell::new(0usize);
    let elapsed = std::cell::Cell::new(Duration::ZERO);
    await_raw_metadata_convergence(
        &expected,
        Duration::from_secs(15),
        Duration::from_secs(5),
        || {
            reads.set(reads.get() + 1);
            responses
                .pop_front()
                .ok_or("raw metadata test response queue exhausted")?
        },
        |delay| elapsed.set(elapsed.get() + delay),
        || elapsed.get(),
    )?;
    assert_eq!(reads.get(), 3, "404 and stale bytes must be retried");
    assert_eq!(elapsed.get(), Duration::from_secs(10));
    Ok(())
}

fn raw_metadata_retry_fails_with_bounded_diagnostics() -> Result<()> {
    let expected = b"expected-metadata";
    let stale = b"permanently-stale-metadata".to_vec();
    let reads = std::cell::Cell::new(0usize);
    let elapsed = std::cell::Cell::new(Duration::ZERO);
    let error = await_raw_metadata_convergence(
        expected,
        Duration::from_secs(10),
        Duration::from_secs(5),
        || {
            reads.set(reads.get() + 1);
            Ok(Some(stale.clone()))
        },
        |delay| elapsed.set(elapsed.get() + delay),
        || elapsed.get(),
    )
    .expect_err("permanently stale public metadata must hit the deadline");
    assert_eq!(reads.get(), 2);
    assert!(
        error
            .to_string()
            .contains(&format!("expected_sha256={}", sha256_hex(expected)))
    );
    assert!(error.to_string().contains("last_http=200"));
    assert!(
        error
            .to_string()
            .contains(&format!("last_observed_sha256={}", sha256_hex(&stale)))
    );
    Ok(())
}

pub(super) fn metadata_token_probe_rejects_wrong_repository_state() -> Result<()> {
    let mut read_only = FakeTransport::new();
    assert!(probe_metadata_token("read", &mut read_only).is_err());
    assert!(read_only.events.is_empty());

    for (bad_ref, bad_bootstrap, bad_target) in [
        (true, false, false),
        (false, true, false),
        (false, false, true),
    ] {
        let mut github = FakeTransport::new();
        github.bad_metadata_ref = bad_ref;
        github.bad_metadata_bootstrap = bad_bootstrap;
        github.bad_metadata_target = bad_target;
        assert!(probe_metadata_token("write", &mut github).is_err());
        assert!(github.events.iter().all(|event| event.starts_with("GET ")));
        assert!(!github.events.iter().any(|event| {
            event.starts_with("PUT ") || event.starts_with("POST ") || event.starts_with("DELETE ")
        }));
    }
    let mut readable = FakeTransport::new();
    probe_metadata_token("write", &mut readable)?;
    for channel in ["stable", "beta"] {
        assert!(readable.events.iter().any(|event| {
            event == &format!(
                "GET repos/{REPOSITORY}/contents/channels/{channel}/latest.json?ref=release-metadata"
            )
        }));
    }
    assert!(
        readable
            .events
            .iter()
            .all(|event| event.starts_with("GET "))
    );
    for channel in ["stable", "beta"] {
        let mut invalid = FakeTransport::new();
        invalid.bad_channel_metadata = Some(channel.to_owned());
        assert!(probe_metadata_token("write", &mut invalid).is_err());
        assert!(invalid.events.iter().all(|event| event.starts_with("GET ")));
    }
    Ok(())
}

pub(super) fn mock_pr_merge_sha_does_not_break_release_or_owned_cleanup() -> Result<()> {
    let (root, options, context, manifest) = fixture()?;
    let runtime = RuntimeIdentity::from_values(
        Some(context.workflow_sha.clone()),
        Some(context.repository.clone()),
        Some(context.run_id.clone()),
        Some(context.run_attempt.to_string()),
        Some("workflow_dispatch".to_owned()),
        Some("refs/heads/main".to_owned()),
        Some(RELEASE_WORKFLOW_REF.to_owned()),
    )?;
    let pr_merge_runtime = RuntimeIdentity {
        github_sha: "0".repeat(40),
        ..runtime
    };
    assert!(
        validate_runtime_identity(&context, Some(&context.workflow_sha), &pr_merge_runtime,)
            .is_err()
    );
    validate_context(&root, &options, &context)?;

    let mut github = FakeTransport::new();
    github.latest = fake_release(11, "v4.1.4", "b".repeat(40), false, false, true);
    let marker = format!(
        "<!-- v4-release-tx: {} -->",
        serde_json::to_string(&json!({
            "repository":context.repository,"run_id":context.run_id,"source_sha":context.source_sha,
            "run_attempt":context.run_attempt,"version":context.version,"tag":context.tag
        }))?
    );
    let mut stale = fake_release(12, &context.tag, &context.source_sha, true, false, false);
    stale["body"] = json!(marker);
    stale["upload_url"] = json!(
        "https://uploads.github.test/repos/pumni/Sky-Auto-Player/releases/12/assets{?name,label}"
    );
    github.releases.insert(12, stale);
    github.next_id = 13;
    prepare_publish_fixture(&options, &context, &manifest, &github.latest)?;
    publish_release_with_mock_verifier(&root, &options, &mut github)?;
    assert_eq!(github.deletions, vec![12]);
    assert_eq!(
        read_json::<ReleaseState>(&options.state_root.join("release-state.json"))?.release_id,
        Some(13)
    );
    let _ = fs::remove_dir_all(&options.state_root);
    Ok(())
}

pub(super) fn mock_conflicting_draft_is_not_deleted() -> Result<()> {
    let (root, options, context, manifest) = fixture()?;
    let mut github = FakeTransport::new();
    github.latest = fake_release(11, "v4.1.4", "b".repeat(40), false, false, true);
    let mut conflict = fake_release(12, &context.tag, &context.source_sha, true, false, false);
    conflict["body"] = json!(format!(
        "<!-- v4-release-tx: {{\"repository\":\"{REPOSITORY}\",\"run_id\":\"other\",\"source_sha\":\"{}\",\"version\":\"{}\",\"tag\":\"{}\"}} -->",
        context.source_sha, context.version, context.tag
    ));
    github.releases.insert(12, conflict);
    github.next_id = 13;
    prepare_publish_fixture(&options, &context, &manifest, &github.latest)?;
    assert!(publish_release_with_mock_verifier(&root, &options, &mut github).is_err());
    assert!(github.deletions.is_empty());
    assert!(!github.events.iter().any(|event| event.starts_with("POST ")));
    let _ = fs::remove_dir_all(&options.state_root);
    Ok(())
}

pub(super) fn mock_digest_mismatch_cleans_owned_draft() -> Result<()> {
    let (root, options, context, manifest) = fixture()?;
    let mut github = FakeTransport::new();
    github.latest = fake_release(11, "v4.1.4", "b".repeat(40), false, false, true);
    github.bad_digest = true;
    prepare_publish_fixture(&options, &context, &manifest, &github.latest)?;
    assert!(publish_release_with_mock_verifier(&root, &options, &mut github).is_err());
    assert_eq!(github.deletions, vec![12]);
    assert!(!github.releases.contains_key(&12));
    let _ = fs::remove_dir_all(&options.state_root);
    Ok(())
}

pub(super) fn mock_partial_upload_cleans_owned_draft() -> Result<()> {
    let (root, options, context, manifest) = fixture()?;
    let mut github = FakeTransport::new();
    github.latest = fake_release(11, "v4.1.4", "b".repeat(40), false, false, true);
    github.fail_upload_after_first = true;
    prepare_publish_fixture(&options, &context, &manifest, &github.latest)?;
    let error = publish_release_with_mock_verifier(&root, &options, &mut github)
        .expect_err("partial asset upload must fail the pre-publication transaction");
    assert!(error.to_string().contains("injected upload failure"));
    assert_eq!(github.deletions, vec![12]);
    assert!(!github.releases.contains_key(&12));
    assert_eq!(
        github
            .events
            .iter()
            .filter(|event| event.starts_with("UPLOAD "))
            .count(),
        2
    );
    assert!(
        !github
            .events
            .iter()
            .any(|event| event.starts_with("PATCH "))
    );
    assert!(!github.events.iter().any(|event| event.starts_with("POST ")
        && (event.contains("/git/refs") || event.contains("tags"))));
    assert_eq!(github.latest["id"], 11);
    let _ = fs::remove_dir_all(&options.state_root);
    Ok(())
}

fn mock_cleanup_failure_preserves_original_error() -> Result<()> {
    let (root, options, context, manifest) = fixture()?;
    let mut github = FakeTransport::new();
    github.latest = fake_release(11, "v4.1.4", "b".repeat(40), false, false, true);
    github.bad_digest = true;
    github.fail_delete = true;
    prepare_publish_fixture(&options, &context, &manifest, &github.latest)?;
    let error = publish_release_with_mock_verifier(&root, &options, &mut github)
        .expect_err("draft cleanup failure must remain visible with the original failure");
    let message = error.to_string();
    assert!(message.contains("GitHub server digest mismatch"));
    assert!(message.contains("owned draft cleanup failed"));
    assert!(message.contains("injected delete failure"));
    assert!(github.releases.contains_key(&12));
    assert!(
        github
            .events
            .iter()
            .all(|event| { event != "PATCH repos/pumni/Sky-Auto-Player/releases/12" })
    );
    let state: ReleaseState = read_json(&options.state_root.join("release-state.json"))?;
    assert_eq!(state.phase, Phase::Ready);
    assert!(!state.qualified_after_download);
    assert_eq!(state.release_id, None);
    let _ = fs::remove_dir_all(&options.state_root);
    Ok(())
}

pub(super) fn mock_patch_timeout_reconciles_exact_release_id() -> Result<()> {
    let (root, options, context, manifest) = fixture()?;
    let mut github = FakeTransport::new();
    github.latest = fake_release(11, "v4.1.4", "b".repeat(40), false, false, true);
    github.patch_applies_then_fails = true;
    prepare_publish_fixture(&options, &context, &manifest, &github.latest)?;
    publish_release_with_mock_verifier(&root, &options, &mut github)?;
    let patch = github
        .events
        .iter()
        .position(|event| event == "PATCH repos/pumni/Sky-Auto-Player/releases/12")
        .ok_or("publication PATCH missing")?;
    assert_eq!(
        github
            .events
            .iter()
            .filter(|event| event.as_str() == "PATCH repos/pumni/Sky-Auto-Player/releases/12")
            .count(),
        1,
        "uncertain PATCH is reconciled, never replayed"
    );
    assert!(github.events.iter().enumerate().any(|(index, event)| {
        index > patch && event == "GET repos/pumni/Sky-Auto-Player/releases/12"
    }));
    assert!(github.deletions.is_empty());
    let state: ReleaseState = read_json(&options.state_root.join("release-state.json"))?;
    assert_eq!(state.phase, Phase::PublishedPendingMetadata);
    assert!(state.qualified_after_download);
    assert_eq!(state.release_id, Some(12));
    assert!(state.published_at.is_some());
    let _ = fs::remove_dir_all(&options.state_root);
    Ok(())
}

pub(super) fn mock_unknown_publication_never_cleans_up() -> Result<()> {
    let (root, options, context, manifest) = fixture()?;
    let mut github = FakeTransport::new();
    github.latest = fake_release(11, "v4.1.4", "b".repeat(40), false, false, true);
    github.fail_reconcile_after_patch = true;
    github.patch_applies_then_fails = true;
    prepare_publish_fixture(&options, &context, &manifest, &github.latest)?;
    let error = publish_release_with_mock_verifier(&root, &options, &mut github)
        .expect_err("unknown publication outcome must surface for operator reconciliation");
    assert!(error.to_string().contains("POST_PUBLICATION_INCIDENT"));
    assert!(github.deletions.is_empty());
    assert_eq!(
        github
            .events
            .iter()
            .filter(|event| event.as_str() == "PATCH repos/pumni/Sky-Auto-Player/releases/12")
            .count(),
        1,
        "unknown PATCH outcome must not be retried"
    );
    let state: ReleaseState = read_json(&options.state_root.join("release-state.json"))?;
    assert_eq!(state.phase, Phase::Ready);
    assert!(state.qualified_after_download);
    assert_eq!(state.release_id, None);
    assert_eq!(state.published_at, None);
    let _ = fs::remove_dir_all(&options.state_root);
    Ok(())
}

pub(super) fn mock_latest_policy_failure_is_post_publication() -> Result<()> {
    let (root, options, context, manifest) = fixture()?;
    let mut github = FakeTransport::new();
    github.latest = fake_release(11, "v4.1.4", "b".repeat(40), false, false, true);
    github.preserve_latest_on_publish = true;
    prepare_publish_fixture(&options, &context, &manifest, &github.latest)?;
    assert!(publish_release_with_mock_verifier(&root, &options, &mut github).is_err());
    let state: ReleaseState = read_json(&options.state_root.join("release-state.json"))?;
    assert_eq!(state.phase, Phase::PublishedPendingMetadata);
    assert!(github.deletions.is_empty());
    let _ = fs::remove_dir_all(&options.state_root);
    Ok(())
}

pub(super) fn mock_stale_evidence_identity_fails_closed() -> Result<()> {
    let (_root, options, context, manifest) = fixture()?;
    let production = options
        .state_root
        .join("candidate-evidence")
        .join(EVIDENCE_NAMES[0]);
    let mut evidence: Value = read_json(&production)?;
    evidence["source_sha"] = json!("e".repeat(40));
    fs::write(&production, serde_json::to_vec(&evidence)?)?;
    assert!(
        validate_candidate_evidence(
            &options.state_root,
            &manifest.qualification_assets,
            &context
        )
        .is_err()
    );
    let mut qualification: Value = read_json(
        &options
            .state_root
            .join("candidate-evidence")
            .join(EVIDENCE_NAMES[1]),
    )?;
    qualification["version"] = json!("4.1.4");
    let qualification_path = options.state_root.join("tampered-qualification.json");
    fs::write(&qualification_path, serde_json::to_vec(&qualification)?)?;
    assert!(
        validate_qualification_evidence(
            &qualification_path,
            &context,
            &manifest.qualification_assets
        )
        .is_err()
    );
    let _ = fs::remove_dir_all(&options.state_root);
    Ok(())
}

pub(super) fn mock_manifest_path_traversal_fails_closed() -> Result<()> {
    let (_root, options, _context, _manifest) = fixture()?;
    let record = CandidateRecord {
        name: "attacker.bin".to_owned(),
        release_name: "attacker.bin".to_owned(),
        source_name: "attacker.bin".to_owned(),
        role: "installer".to_owned(),
        size: 1,
        sha256: "0".repeat(64),
        state_path: "../outside.bin".to_owned(),
    };
    assert!(safe_state_asset_path(&options.state_root, &record).is_err());
    let _ = fs::remove_dir_all(&options.state_root);
    Ok(())
}

pub(super) fn marker_parser_rejects_duplicates_and_multiple_markers() -> Result<()> {
    let (_, options, context, _) = fixture()?;
    let mut release = fake_release(12, &context.tag, &context.source_sha, true, false, false);
    release["body"] = json!(format!(
        "<!-- v4-release-tx: {{\"repository\":\"{REPOSITORY}\",\"repository\":\"{REPOSITORY}\",\"run_id\":\"run\",\"source_sha\":\"{}\",\"version\":\"{}\",\"tag\":\"{}\"}} -->",
        context.source_sha, context.version, context.tag
    ));
    assert!(!matches_transaction(&release, &context));
    release["body"] = json!(format!(
        "<!-- v4-release-tx: {{\"repository\":\"{REPOSITORY}\",\"run_id\":\"run\",\"source_sha\":\"{}\",\"version\":\"{}\",\"tag\":\"{}\"}} --><!-- v4-release-tx: {{}} -->",
        context.source_sha, context.version, context.tag
    ));
    assert!(!matches_transaction(&release, &context));
    let _ = fs::remove_dir_all(&options.state_root);
    Ok(())
}

pub(super) fn workflow_run_identity_fails_closed() -> Result<()> {
    let good = json!({"path":".github/workflows/release-v4.yml@refs/heads/main","event":"workflow_dispatch","head_sha":"a".repeat(40),"repository":{"full_name":REPOSITORY}});
    assert!(valid_workflow_run(&good, Some(&"a".repeat(40)), REPOSITORY));
    for bad in [
        json!({"path":".github/workflows/rehearse-v4.yml","event":"workflow_dispatch","head_sha":"a".repeat(40),"repository":{"full_name":REPOSITORY}}),
        json!({"path":"attacker/.github/workflows/release-v4.yml@refs/heads/main","event":"workflow_dispatch","head_sha":"a".repeat(40),"repository":{"full_name":REPOSITORY}}),
        json!({"path":".github/workflows/release-v4.yml","event":"push","head_sha":"a".repeat(40),"repository":{"full_name":REPOSITORY}}),
        json!({"path":".github/workflows/release-v4.yml","event":"workflow_dispatch","head_sha":"b".repeat(40),"repository":{"full_name":REPOSITORY}}),
        json!({"path":".github/workflows/release-v4.yml","event":"workflow_dispatch","head_sha":"a".repeat(40)}),
    ] {
        assert!(!valid_workflow_run(&bad, Some(&"a".repeat(40)), REPOSITORY));
    }
    Ok(())
}

fn runtime_identity_fails_closed_without_exact_workflow_context() -> Result<()> {
    let (_, options, context, _) = fixture()?;
    let alternate_run_id = format!("{}0", context.run_id);
    let alternate_attempt = if context.run_attempt == 1 { "2" } else { "1" };
    let valid = RuntimeIdentity::from_values(
        Some(context.workflow_sha.clone()),
        Some(context.repository.clone()),
        Some(context.run_id.clone()),
        Some(context.run_attempt.to_string()),
        Some("workflow_dispatch".to_owned()),
        Some("refs/heads/main".to_owned()),
        Some(RELEASE_WORKFLOW_REF.to_owned()),
    )?;
    assert!(validate_runtime_identity(&context, Some(&context.workflow_sha), &valid).is_ok());
    assert!(validate_runtime_identity(&context, None, &valid).is_err());
    assert!(
        RuntimeIdentity::from_values(
            None,
            Some(context.repository.clone()),
            Some(context.run_id.clone()),
            Some(context.run_attempt.to_string()),
            Some("workflow_dispatch".to_owned()),
            Some("refs/heads/main".to_owned()),
            Some(RELEASE_WORKFLOW_REF.to_owned()),
        )
        .is_err()
    );
    for invalid in [
        RuntimeIdentity {
            github_sha: "f".repeat(40),
            ..valid.clone()
        },
        RuntimeIdentity {
            github_repository: "attacker/other".to_owned(),
            ..valid.clone()
        },
        RuntimeIdentity {
            github_run_id: alternate_run_id,
            ..valid.clone()
        },
        RuntimeIdentity {
            github_run_attempt: alternate_attempt.to_owned(),
            ..valid.clone()
        },
        RuntimeIdentity {
            github_event_name: "push".to_owned(),
            ..valid.clone()
        },
        RuntimeIdentity {
            github_ref: "refs/heads/feature".to_owned(),
            ..valid.clone()
        },
        RuntimeIdentity {
            github_workflow_ref: format!("{RELEASE_WORKFLOW_REF}@refs/heads/feature"),
            ..valid.clone()
        },
    ] {
        assert!(
            validate_runtime_identity(&context, Some(&context.workflow_sha), &invalid).is_err()
        );
    }
    let _ = fs::remove_dir_all(&options.state_root);
    Ok(())
}

pub(super) fn timestamp_normalization_is_utc_and_strict() -> Result<()> {
    assert_eq!(
        canonical_timestamp("2026-10-25T12:34:56+02:30")?,
        "2026-10-25T10:04:56Z"
    );
    assert_eq!(
        canonical_timestamp("2026-01-01T00:00:00.123Z")?,
        "2026-01-01T00:00:00Z"
    );
    assert!(canonical_timestamp("not-a-date").is_err());
    Ok(())
}

#[derive(Default)]
pub(super) struct FakeTransport {
    pub(super) latest: Value,
    pub(super) releases: std::collections::BTreeMap<u64, Value>,
    pub(super) next_id: u64,
    pub(super) assets: std::collections::BTreeMap<String, Vec<u8>>,
    pub(super) events: Vec<String>,
    pub(super) deletions: Vec<u64>,
    pub(super) bad_digest: bool,
    pub(super) fail_download: bool,
    pub(super) corrupt_download: bool,
    pub(super) corrupt_signature_download: bool,
    pub(super) patch_applies_then_fails: bool,
    pub(super) fail_reconcile_after_patch: bool,
    pub(super) preserve_latest_on_publish: bool,
    pub(super) fail_upload_after_first: bool,
    pub(super) fail_upload_after_commit: bool,
    pub(super) fail_delete: bool,
    pub(super) post_created_then_timeout: bool,
    pub(super) fail_metadata_put_after_write: bool,
    pub(super) fail_metadata_put_without_write: bool,
    pub(super) bad_metadata_ref: bool,
    pub(super) bad_metadata_bootstrap: bool,
    pub(super) bad_metadata_target: bool,
    pub(super) bad_channel_metadata: Option<String>,
    pub(super) metadata: Vec<u8>,
    pub(super) metadata_files: std::collections::BTreeMap<String, Vec<u8>>,
    pub(super) raw_metadata_files: std::collections::BTreeMap<String, Vec<u8>>,
    pub(super) workflow_runs: std::collections::BTreeMap<String, Value>,
    pub(super) workflow_jobs: std::collections::BTreeMap<(String, u32), Vec<Value>>,
}

impl FakeTransport {
    pub(super) fn new() -> Self {
        Self {
            next_id: 12,
            ..Self::default()
        }
    }
}

impl Transport for FakeTransport {
    fn request(
        &mut self,
        method: &str,
        endpoint: &str,
        body: Option<&Value>,
    ) -> Result<Option<Value>> {
        self.events.push(format!("{method} {endpoint}"));
        if method == "GET" && endpoint.starts_with("repos/pumni/Sky-Auto-Player/actions/runs/") {
            let path = endpoint
                .strip_prefix("repos/pumni/Sky-Auto-Player/actions/runs/")
                .unwrap_or_default();
            if let Some((run_id, attempt_query)) = path.split_once("/attempts/") {
                let Some((attempt, query)) = attempt_query.split_once("/jobs?") else {
                    return Err("fake workflow job endpoint omitted attempt jobs path".into());
                };
                let run_attempt = attempt.parse::<u32>()?;
                let query_value = |name: &str| {
                    query
                        .split('&')
                        .find_map(|part| part.strip_prefix(&format!("{name}=")))
                        .and_then(|value| value.parse::<usize>().ok())
                };
                let per_page = query_value("per_page").ok_or("fake jobs page size missing")?;
                let page = query_value("page").ok_or("fake jobs page number missing")?;
                if per_page == 0 || page == 0 {
                    return Err("fake jobs pagination is invalid".into());
                }
                let Some(jobs) = self.workflow_jobs.get(&(run_id.to_owned(), run_attempt)) else {
                    return Ok(None);
                };
                let start = (page - 1).saturating_mul(per_page);
                let items = jobs
                    .iter()
                    .skip(start)
                    .take(per_page)
                    .cloned()
                    .collect::<Vec<_>>();
                return Ok(Some(json!({"total_count":jobs.len(),"jobs":items})));
            }
            let id = path.split('?').next().unwrap_or_default();
            return Ok(self.workflow_runs.get(id).cloned());
        }
        if method == "GET" && endpoint == "repos/pumni/Sky-Auto-Player" {
            let target = if self.bad_metadata_target {
                "pumni/Other-Repository"
            } else {
                REPOSITORY
            };
            return Ok(Some(json!({"full_name":target})));
        }
        if method == "GET" && endpoint == "repos/pumni/Sky-Auto-Player/releases/latest" {
            return Ok(Some(self.latest.clone()));
        }
        if method == "GET"
            && endpoint == "repos/pumni/Sky-Auto-Player/git/ref/heads/release-metadata"
        {
            let expected_ref = if self.bad_metadata_ref {
                "refs/heads/main"
            } else {
                "refs/heads/release-metadata"
            };
            return Ok(Some(
                json!({"ref":expected_ref,"object":{"sha":"a".repeat(40)}}),
            ));
        }
        if method == "GET"
            && endpoint
                == "repos/pumni/Sky-Auto-Player/contents/.release-metadata/README.md?ref=release-metadata"
        {
            let bootstrap = if self.bad_metadata_bootstrap {
                "wrong repository or branch"
            } else {
                BOOTSTRAP_CONTENT
            };
            return Ok(Some(
                json!({"encoding":"base64","content":base64::engine::general_purpose::STANDARD.encode(bootstrap),"sha":"b".repeat(40)}),
            ));
        }
        if endpoint.starts_with("repos/pumni/Sky-Auto-Player/contents/channels/") {
            let channel = endpoint
                .strip_prefix("repos/pumni/Sky-Auto-Player/contents/channels/")
                .and_then(|value| value.split('/').next())
                .ok_or("fake metadata channel missing")?;
            if !matches!(channel, "stable" | "beta") {
                return Err("fake metadata request had an unsupported channel".into());
            }
            match method {
                "GET" => {
                    if self.bad_channel_metadata.as_deref() == Some(channel) {
                        return Ok(Some(json!({
                            "encoding":"base64",
                            "content":base64::engine::general_purpose::STANDARD.encode(b"not valid release metadata"),
                            "sha":"d".repeat(40)
                        })));
                    }
                    return Ok(self.metadata_files.get(channel).map(|bytes| {
                        json!({"encoding":"base64","content":base64::engine::general_purpose::STANDARD.encode(bytes),"sha":"c".repeat(40)})
                    }));
                }
                "PUT" => {
                    if self.fail_metadata_put_without_write {
                        self.fail_metadata_put_without_write = false;
                        return Err(Box::new(HttpFailure {
                            status: Some(504),
                            detail: "injected metadata write timeout before commit".to_owned(),
                        }));
                    }
                    let payload = body.ok_or("fake metadata PUT body missing")?;
                    let encoded = payload["content"]
                        .as_str()
                        .ok_or("fake metadata PUT content missing")?;
                    let bytes = base64::engine::general_purpose::STANDARD.decode(encoded)?;
                    self.metadata = bytes.clone();
                    self.metadata_files.insert(channel.to_owned(), bytes);
                    if let Some(contents) = self.metadata_files.get(channel).cloned() {
                        self.raw_metadata_files.insert(channel.to_owned(), contents);
                    }
                    if self.fail_metadata_put_after_write {
                        self.fail_metadata_put_after_write = false;
                        return Err(Box::new(HttpFailure {
                            status: Some(504),
                            detail: "injected metadata write timeout after commit".to_owned(),
                        }));
                    }
                    return Ok(Some(
                        json!({"content":{"path":endpoint,"sha":"c".repeat(40)}}),
                    ));
                }
                _ => {}
            }
        }
        if method == "GET" && endpoint.starts_with("repos/pumni/Sky-Auto-Player/releases/tags/") {
            let tag = endpoint.rsplit('/').next().unwrap_or_default();
            return Ok(self
                .releases
                .values()
                .find(|release| release["tag_name"] == tag)
                .cloned());
        }
        if let Some(id_text) = endpoint.strip_prefix("repos/pumni/Sky-Auto-Player/releases/")
            && let Ok(id) = id_text.parse::<u64>()
        {
            match method {
                "GET" => {
                    if self.fail_reconcile_after_patch
                        && self
                            .releases
                            .get(&id)
                            .is_some_and(|release| release["draft"] == false)
                    {
                        return Err(Box::new(HttpFailure {
                            status: Some(503),
                            detail: "injected read failure".to_owned(),
                        }));
                    }
                    return Ok(self.releases.get(&id).cloned());
                }
                "DELETE" => {
                    self.deletions.push(id);
                    if self.fail_delete {
                        return Err(Box::new(HttpFailure {
                            status: Some(403),
                            detail: "injected delete failure".to_owned(),
                        }));
                    }
                    self.releases.remove(&id);
                    return Ok(None);
                }
                "PATCH" => {
                    let release = self.releases.get_mut(&id).ok_or("release missing")?;
                    release["draft"] = body.ok_or("PATCH body missing")?["draft"].clone();
                    if release["draft"] == false {
                        release["immutable"] = json!(true);
                        release["published_at"] = json!("2026-10-25T09:00:00Z");
                        if !self.preserve_latest_on_publish {
                            self.latest = release.clone();
                        }
                    }
                    if self.patch_applies_then_fails {
                        self.patch_applies_then_fails = false;
                        return Err(Box::new(HttpFailure {
                            status: Some(504),
                            detail: "injected patch timeout".to_owned(),
                        }));
                    }
                    return Ok(Some(release.clone()));
                }
                _ => {}
            }
        }
        if method == "POST" && endpoint == "repos/pumni/Sky-Auto-Player/releases" {
            let body = body.ok_or("draft body missing")?;
            let id = self.next_id;
            self.next_id += 1;
            let release = json!({"id":id,"tag_name":body["tag_name"],"target_commitish":body["target_commitish"],
                "name":body["name"],"body":body["body"],"draft":true,"prerelease":body["prerelease"],"immutable":false,
                "upload_url":format!("https://uploads.github.test/repos/{REPOSITORY}/releases/{id}/assets{{?name,label}}"),"assets":[]});
            self.releases.insert(id, release.clone());
            if self.post_created_then_timeout {
                self.post_created_then_timeout = false;
                return Err(Box::new(HttpFailure {
                    status: Some(504),
                    detail: "injected draft POST timeout after commit".to_owned(),
                }));
            }
            return Ok(Some(release));
        }
        if method == "GET" && endpoint.starts_with("repos/pumni/Sky-Auto-Player/git/ref/tags/") {
            return Ok(None);
        }
        Err(format!("unhandled fake GitHub request: {method} {endpoint}").into())
    }

    fn upload(&mut self, endpoint: &str, name: &str, path: &Path) -> Result<()> {
        self.events.push(format!("UPLOAD {name}"));
        if self.fail_upload_after_first
            && self
                .events
                .iter()
                .filter(|event| event.starts_with("UPLOAD "))
                .count()
                > 1
        {
            return Err("injected upload failure after partial publication".into());
        }
        let id = endpoint
            .split("/releases/")
            .nth(1)
            .and_then(|value| value.split('/').next())
            .ok_or("fake upload release id missing")?
            .parse::<u64>()?;
        let bytes = fs::read(path)?;
        let digest = if self.bad_digest {
            "sha256:".to_owned() + &"0".repeat(64)
        } else {
            format!("sha256:{}", sha256_hex(&bytes))
        };
        let asset_index = self
            .releases
            .get(&id)
            .and_then(|release| release["assets"].as_array())
            .map(Vec::len)
            .ok_or("fake release assets missing")?;
        let asset_id = id
            .checked_mul(1000)
            .and_then(|value| value.checked_add(asset_index as u64 + 1))
            .ok_or("fake asset identifier overflow")?;
        let url = format!("https://api.github.com/repos/{REPOSITORY}/releases/assets/{asset_id}");
        self.assets.insert(url.clone(), bytes.clone());
        let release = self.releases.get_mut(&id).ok_or("fake release missing")?;
        let assets = release["assets"]
            .as_array_mut()
            .ok_or("fake assets missing")?;
        assets.push(
            json!({"name":name,"size":bytes.len(),"state":"uploaded","digest":digest,"url":url}),
        );
        if self.fail_upload_after_commit {
            self.fail_upload_after_commit = false;
            return Err(Box::new(HttpFailure {
                status: Some(504),
                detail: "injected upload timeout after commit".to_owned(),
            }));
        }
        Ok(())
    }

    fn download(&mut self, endpoint: &str, destination: &Path) -> Result<()> {
        self.events.push(format!("DOWNLOAD {endpoint}"));
        if self.fail_download {
            self.fail_download = false;
            return Err("injected draft asset download failure".into());
        }
        if self.corrupt_signature_download
            && self.releases.values().any(|release| {
                release["assets"].as_array().is_some_and(|assets| {
                    assets.iter().any(|asset| {
                        asset["url"] == endpoint
                            && asset["name"]
                                .as_str()
                                .is_some_and(|name| name.ends_with(".sig"))
                    })
                })
            })
        {
            self.corrupt_signature_download = false;
            fs::write(destination, b"corrupted draft signature")?;
            return Ok(());
        }
        if self.corrupt_download {
            self.corrupt_download = false;
            fs::write(destination, b"corrupted draft asset")?;
            return Ok(());
        }
        let bytes = self.assets.get(endpoint).ok_or("fake asset not found")?;
        fs::write(destination, bytes)?;
        Ok(())
    }

    fn raw_get(&mut self, endpoint: &str) -> Result<Option<Vec<u8>>> {
        self.events.push(format!("RAW_GET {endpoint}"));
        let channel = endpoint
            .rsplit("/channels/")
            .next()
            .and_then(|value| value.strip_suffix("/latest.json"))
            .ok_or("fake raw metadata URL was not canonical")?;
        Ok(self.raw_metadata_files.get(channel).cloned())
    }
}

pub(super) fn fake_release(
    id: u64,
    tag: &str,
    source: impl AsRef<str>,
    draft: bool,
    prerelease: bool,
    immutable: bool,
) -> Value {
    json!({"id":id,"tag_name":tag,"target_commitish":source.as_ref(),"draft":draft,"prerelease":prerelease,
        "immutable":immutable,"published_at":if draft { Value::Null } else { json!("2026-01-01T00:00:00Z") },"assets":[]})
}

pub(super) fn fixture() -> Result<(PathBuf, Options, Context, CandidateManifest)> {
    let root = repo::root();
    let version = repo::project_version(&root)?;
    let parsed = version::parse(&version)?;
    let channel = if parsed.pre.is_empty() {
        "stable"
    } else {
        "beta"
    };
    let source_sha = git_text(&root, &["rev-parse", "HEAD"])?;
    let tag = format!("v{version}");
    let context = Context {
        schema_version: 2,
        repository: REPOSITORY.to_owned(),
        version: version.clone(),
        channel: channel.to_owned(),
        tag: tag.clone(),
        release_notes_path: format!("docs/releases/{tag}.md"),
        source_sha: source_sha.clone(),
        workflow_sha: source_sha.clone(),
        run_id: env::var("GITHUB_RUN_ID").unwrap_or_else(|_| "9001".to_owned()),
        run_attempt: env::var("GITHUB_RUN_ATTEMPT")
            .ok()
            .and_then(|value| value.parse().ok())
            .filter(|value| *value > 0 && *value <= 100)
            .unwrap_or(1),
        created_at: "2026-10-25T00:00:00Z".to_owned(),
    };
    static NEXT_FIXTURE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let fixture_id = NEXT_FIXTURE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let state_root = env::temp_dir().join(format!(
        "sky-v4-release-mock-{}-{fixture_id}",
        std::process::id()
    ));
    fs::create_dir_all(state_root.join("candidate-bundle"))?;
    fs::create_dir_all(state_root.join("candidate-evidence"))?;
    let options = Options {
        state_root: state_root.clone(),
        workflow_sha: None,
    };
    let installer_name = release_metadata::canonical_installer_name(&version);
    let signature_name = format!("{installer_name}.sig");
    let mut base = Vec::new();
    for (name, role, dir, bytes) in [
        (
            installer_name.as_str(),
            "installer",
            "candidate-bundle",
            b"installer-fixture".as_slice(),
        ),
        (
            signature_name.as_str(),
            "updater-signature",
            "candidate-bundle",
            b"c2lnbmF0dXJl".as_slice(),
        ),
        (
            EVIDENCE_NAMES[2],
            "authenticode-evidence",
            "candidate-evidence",
            b"auth evidence".as_slice(),
        ),
        (
            EVIDENCE_NAMES[5],
            "sbom",
            "candidate-evidence",
            b"sbom fixture".as_slice(),
        ),
    ] {
        let path = state_root.join(dir).join(name);
        fs::write(&path, bytes)?;
        base.push(CandidateRecord {
            name: name.replace(' ', "."),
            release_name: name.replace(' ', "."),
            source_name: name.to_owned(),
            role: role.to_owned(),
            size: bytes.len() as u64,
            sha256: sha256_hex(bytes),
            state_path: Path::new(dir)
                .join(name)
                .to_string_lossy()
                .replace('\\', "/"),
        });
    }
    let installer = base
        .iter()
        .find(|record| record.role == "installer")
        .unwrap()
        .clone();
    let signature = base
        .iter()
        .find(|record| record.role == "updater-signature")
        .unwrap()
        .clone();
    let auth = base
        .iter()
        .find(|record| record.role == "authenticode-evidence")
        .unwrap()
        .clone();
    let sbom = base
        .iter()
        .find(|record| record.role == "sbom")
        .unwrap()
        .clone();
    let production = json!({"repository":context.repository,"source_sha":source_sha,"workflow_sha":context.workflow_sha,"run_id":context.run_id,"run_attempt":context.run_attempt,"version":version,"channel":channel,"authenticode_mode":"unsigned-zero-budget","authenticode_state":"unsigned","authenticode_provider":"none","approved_signer_thumbprint":null,"observed_signer_thumbprint":null,
        "updater_signature_status":"valid","updater_key_id":crate::updater_trust::canonical_key_id()?,"qualification_status":"PASS","installer":installer.source_name,"installer_size":installer.size,"installer_sha256":installer.sha256,
        "updater_signature":signature.source_name,"signature_size":signature.size,"updater_signature_sha256":signature.sha256,"authenticode_evidence":auth.source_name,"authenticode_evidence_sha256":auth.sha256,"sbom":sbom.source_name,"sbom_sha256":sbom.sha256});
    let qualification = json!({"schema_version":1,"evidence_type":"tauri-nsis-qualified-release","repository":context.repository,"source_sha":context.source_sha,"workflow_sha":context.workflow_sha,"run_id":context.run_id,"run_attempt":context.run_attempt,"qualified":true,"qualification":"install-launch-uninstall","product_name":"Sky Auto Player","identifier":"io.github.pumni.skyautoplayer","version":version,"target":"nsis","install_mode":"currentUser","installer":installer.source_name,"updater_signature":signature.source_name,"installer_size":installer.size,"signature_size":signature.size,"installer_sha256":installer.sha256,"updater_signature_sha256":signature.sha256,"authenticode_mode":"unsigned-zero-budget","authenticode_evidence":auth.source_name,"authenticode_evidence_sha256":auth.sha256,"sbom":sbom.source_name,"sbom_sha256":sbom.sha256});
    for (name, value) in [
        (EVIDENCE_NAMES[0], &production),
        (EVIDENCE_NAMES[1], &qualification),
    ] {
        let path = state_root.join("candidate-evidence").join(name);
        fs::write(&path, serde_json::to_vec(value)?)?;
        let bytes = fs::read(&path)?;
        base.push(CandidateRecord {
            name: name.to_owned(),
            release_name: name.to_owned(),
            source_name: name.to_owned(),
            role: if name == EVIDENCE_NAMES[0] {
                "production-evidence"
            } else {
                "qualification-evidence"
            }
            .to_owned(),
            size: bytes.len() as u64,
            sha256: sha256_hex(&bytes),
            state_path: format!("candidate-evidence/{name}"),
        });
    }
    for name in [EVIDENCE_NAMES[3], EVIDENCE_NAMES[4]] {
        let path = state_root.join("candidate-evidence").join(name);
        fs::write(&path, b"fixture evidence")?;
        let bytes = fs::read(&path)?;
        base.push(CandidateRecord {
            name: name.to_owned(),
            release_name: name.to_owned(),
            source_name: name.to_owned(),
            role: if name == EVIDENCE_NAMES[3] {
                "installed-authenticode-evidence"
            } else {
                "artifact-summary"
            }
            .to_owned(),
            size: bytes.len() as u64,
            sha256: sha256_hex(&bytes),
            state_path: format!("candidate-evidence/{name}"),
        });
    }
    base.sort_by(|a, b| a.source_name.cmp(&b.source_name));
    validate_candidate_evidence(&state_root, &base, &context)?;
    let public_names = [
        release_metadata::canonical_release_installer_name(&version),
        format!(
            "{}.sig",
            release_metadata::canonical_release_installer_name(&version)
        ),
    ];
    let public_assets = base
        .iter()
        .filter(|record| public_names.contains(&record.release_name))
        .cloned()
        .collect();
    let installer_record = base
        .iter()
        .find(|record| record.role == "installer")
        .ok_or("mock fixture installer record missing")?;
    let unsigned_handoff = release_handoff::UnsignedArtifactManifest {
        schema_version: 3,
        repository: context.repository.clone(),
        source_sha: context.source_sha.clone(),
        workflow_sha: context.workflow_sha.clone(),
        run_id: context.run_id.clone(),
        run_attempt: context.run_attempt,
        version: context.version.clone(),
        installer_name: installer_record.source_name.clone(),
        installer_size: installer_record.size,
        installer_sha256: installer_record.sha256.clone(),
        producer_job: "build_unsigned".to_owned(),
        artifact_name: release_handoff::artifact_name(&context.run_id, context.run_attempt)?,
    };
    Ok((
        root,
        options,
        context.clone(),
        CandidateManifest {
            schema_version: 2,
            repository: context.repository.clone(),
            source_sha: context.source_sha.clone(),
            workflow_sha: context.workflow_sha.clone(),
            run_id: context.run_id.clone(),
            run_attempt: context.run_attempt,
            version,
            channel: channel.to_owned(),
            tag,
            unsigned_handoff,
            qualification_assets: base,
            public_assets,
        },
    ))
}

pub(super) fn fixture_state(context: &Context) -> ReleaseState {
    ReleaseState {
        schema_version: 2,
        repository: context.repository.clone(),
        version: context.version.clone(),
        channel: context.channel.clone(),
        tag: context.tag.clone(),
        source_sha: context.source_sha.clone(),
        workflow_sha: context.workflow_sha.clone(),
        run_id: context.run_id.clone(),
        run_attempt: context.run_attempt,
        phase: Phase::Ready,
        // Publish tests begin after the separately guarded exact-source attestation phase.
        attested: true,
        qualified_after_download: false,
        release_id: None,
        published_at: None,
        promoted_at: None,
        final_verified: false,
    }
}

pub(super) fn prepare_publish_fixture(
    options: &Options,
    context: &Context,
    manifest: &CandidateManifest,
    latest: &Value,
) -> Result<()> {
    write_json_atomic(&options.state_root.join("release-context.json"), context)?;
    write_json_atomic(
        &options.state_root.join("release-state.json"),
        &fixture_state(context),
    )?;
    write_json_atomic(
        &options.state_root.join("candidate-manifest.json"),
        manifest,
    )?;
    write_json_atomic(
        &options.state_root.join("preflight-evidence.json"),
        &json!({"pre_publication_latest":latest}),
    )?;
    Ok(())
}
