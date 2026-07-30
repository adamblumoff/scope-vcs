CREATE TABLE scope_auth_identities (
    provider character varying NOT NULL,
    subject character varying NOT NULL,
    user_id character varying NOT NULL
);


--
-- Name: scope_cli_browser_logins; Type: TABLE; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

CREATE TABLE scope_cli_browser_logins (
    request_id character varying NOT NULL,
    request_secret_hash character varying NOT NULL,
    callback_url text NOT NULL,
    callback_code_hash character varying,
    created_at_unix bigint NOT NULL,
    expires_at_unix bigint NOT NULL,
    completed_user_id character varying,
    completed_at_unix bigint,
    consumed_at_unix bigint
);


--
-- Name: scope_cli_device_logins; Type: TABLE; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

CREATE TABLE scope_cli_device_logins (
    device_code_hash character varying NOT NULL,
    user_code_hash character varying NOT NULL,
    created_at_unix bigint NOT NULL,
    expires_at_unix bigint NOT NULL,
    completed_user_id character varying,
    completed_at_unix bigint,
    consumed_at_unix bigint
);


--
-- Name: scope_cli_exchange_grants; Type: TABLE; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

CREATE TABLE scope_cli_exchange_grants (
    grant_hash character varying NOT NULL,
    user_id character varying NOT NULL,
    created_at_unix bigint NOT NULL,
    expires_at_unix bigint NOT NULL,
    consumed_at_unix bigint
);


--
-- Name: scope_cli_sessions; Type: TABLE; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

CREATE TABLE scope_cli_sessions (
    id character varying NOT NULL,
    token_hash character varying NOT NULL,
    user_id character varying NOT NULL,
    label character varying NOT NULL,
    created_at_unix bigint NOT NULL,
    last_used_at_unix bigint,
    expires_at_unix bigint NOT NULL,
    revoked_at_unix bigint
);


--
-- Name: scope_credit_ledger_entries; Type: TABLE; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

CREATE TABLE scope_credit_ledger_entries (
    id character varying NOT NULL,
    user_id character varying NOT NULL,
    request_id character varying,
    kind character varying NOT NULL,
    amount_credits integer NOT NULL,
    created_at_unix bigint NOT NULL
);


--
-- Name: scope_metadata_locks; Type: TABLE; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

CREATE TABLE scope_metadata_locks (
    key character varying NOT NULL
);


CREATE TABLE scope_metadata_schema (
    key character varying NOT NULL,
    version bigint NOT NULL,
    deploy_revision character varying NOT NULL,
    ready boolean NOT NULL,
    PRIMARY KEY (key)
);


--
-- Name: scope_metadata_reset_events; Type: TABLE; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

CREATE TABLE scope_metadata_reset_events (
    id character varying NOT NULL,
    reset_at_unix bigint NOT NULL,
    trigger character varying NOT NULL,
    reason text NOT NULL
);


--
-- Name: scope_outbox_jobs; Type: TABLE; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

CREATE TABLE scope_outbox_jobs (
    id character varying NOT NULL,
    idempotency_key character varying NOT NULL,
    kind character varying NOT NULL,
    repo_id character varying NOT NULL,
    repo_version bigint NOT NULL,
    payload jsonb NOT NULL,
    state character varying NOT NULL,
    attempts bigint NOT NULL,
    next_run_at_unix bigint NOT NULL,
    lease_owner character varying,
    lease_expires_at_unix bigint,
    last_error text,
    created_at_unix bigint NOT NULL,
    updated_at_unix bigint NOT NULL,
    completed_at_unix bigint
);

CREATE TABLE scope_runners (
    id character varying PRIMARY KEY,
    owner_user_id character varying NOT NULL,
    secret_hash character varying NOT NULL UNIQUE,
    version character varying NOT NULL,
    protocol_version integer NOT NULL,
    capabilities jsonb NOT NULL,
    enabled boolean NOT NULL,
    created_at_unix bigint NOT NULL,
    last_seen_at_unix bigint
);

CREATE TABLE scope_runner_grants (
    repo_id character varying NOT NULL,
    runner_id character varying NOT NULL,
    name character varying NOT NULL,
    granted_by_user_id character varying NOT NULL,
    created_at_unix bigint NOT NULL,
    revoked_at_unix bigint,
    PRIMARY KEY (repo_id, runner_id)
);

CREATE TABLE scope_workflow_revisions (
    digest character varying PRIMARY KEY,
    definition jsonb NOT NULL,
    created_at_unix bigint NOT NULL
);

CREATE TABLE scope_push_trigger_evaluations (
    repo_id character varying NOT NULL,
    change_version bigint NOT NULL,
    head_oid character varying NOT NULL,
    state character varying NOT NULL,
    message text,
    checks jsonb NOT NULL,
    created_at_unix bigint NOT NULL,
    completed_at_unix bigint,
    PRIMARY KEY (repo_id, change_version)
);

CREATE TABLE scope_runs (
    id character varying PRIMARY KEY,
    idempotency_key character varying NOT NULL,
    repo_id character varying NOT NULL,
    workflow_path text NOT NULL,
    workflow_revision_digest character varying NOT NULL,
    trigger character varying NOT NULL,
    requested_by_user_id character varying,
    source jsonb NOT NULL,
    pinned_container_image text,
    desired_runner_name character varying,
    state character varying NOT NULL,
    cancellation_requested boolean NOT NULL,
    last_attempt_number integer NOT NULL,
    current_attempt_id character varying,
    created_at_unix bigint NOT NULL,
    updated_at_unix bigint NOT NULL,
    completed_at_unix bigint,
    UNIQUE (repo_id, idempotency_key)
);

CREATE TABLE scope_run_attempts (
    id character varying PRIMARY KEY,
    run_id character varying NOT NULL,
    number integer NOT NULL,
    runner_id character varying NOT NULL,
    runner_name character varying NOT NULL,
    token_hash character varying NOT NULL UNIQUE,
    token_expires_at_unix bigint NOT NULL,
    state character varying NOT NULL,
    lease_expires_at_unix bigint NOT NULL,
    last_heartbeat_at_unix bigint NOT NULL,
    created_at_unix bigint NOT NULL,
    started_at_unix bigint,
    completed_at_unix bigint,
    terminal_reason jsonb,
    log_bytes bigint NOT NULL,
    logs_truncated boolean NOT NULL,
    UNIQUE (run_id, number)
);

CREATE TABLE scope_run_attempt_steps (
    attempt_id character varying NOT NULL,
    step_index integer NOT NULL,
    state character varying NOT NULL,
    started_at_unix bigint,
    completed_at_unix bigint,
    exit_code integer,
    PRIMARY KEY (attempt_id, step_index)
);

CREATE TABLE scope_run_logs (
    position bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    run_id character varying NOT NULL,
    attempt_id character varying NOT NULL,
    step_index integer NOT NULL,
    sequence bigint NOT NULL,
    text text NOT NULL,
    created_at_unix bigint NOT NULL,
    UNIQUE (attempt_id, sequence)
);


--
-- Name: scope_projection_files; Type: TABLE; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

CREATE TABLE scope_projection_files (
    repo_id character varying NOT NULL,
    repo_version bigint NOT NULL,
    source character varying NOT NULL,
    audience character varying NOT NULL,
    path_key character varying NOT NULL,
    path character varying NOT NULL,
    oid character varying NOT NULL,
    visibility character varying NOT NULL,
    object_key character varying NOT NULL,
    sha256 character varying NOT NULL,
    size_bytes bigint NOT NULL,
    git_file_mode character varying NOT NULL
);


--
-- Name: scope_projection_read_models; Type: TABLE; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

CREATE TABLE scope_projection_read_models (
    repo_id character varying NOT NULL,
    repo_version bigint NOT NULL,
    source character varying NOT NULL,
    audience character varying NOT NULL,
    rebuilt_at_unix bigint NOT NULL,
    file_count bigint NOT NULL
);


--
-- Name: scope_repo_storage_cleanup_jobs; Type: TABLE; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

CREATE TABLE scope_repo_storage_cleanup_jobs (
    repo_id character varying NOT NULL,
    generation character varying NOT NULL,
    owner_handle character varying NOT NULL,
    repo_name character varying NOT NULL,
    attempts integer NOT NULL,
    next_run_at_unix bigint NOT NULL,
    last_error text,
    completed_at_unix bigint,
    created_at_unix bigint NOT NULL,
    updated_at_unix bigint NOT NULL
);


--
-- Name: scope_repositories; Type: TABLE; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

CREATE TABLE scope_repositories (
    id character varying NOT NULL,
    owner_handle character varying NOT NULL,
    name character varying NOT NULL,
    owner_user_id character varying NOT NULL,
    publication_state character varying NOT NULL,
    default_visibility character varying NOT NULL,
    change_version bigint NOT NULL,
    repo_config jsonb NOT NULL,
    policy jsonb NOT NULL
);

CREATE TABLE scope_logical_commits (
    id character varying NOT NULL,
    repo_id character varying NOT NULL,
    ordinal bigint NOT NULL,
    origin jsonb NOT NULL,
    author_id character varying NOT NULL,
    message text NOT NULL,
    PRIMARY KEY (repo_id, id),
    UNIQUE (repo_id, ordinal)
);

CREATE TABLE scope_file_changes (
    repo_id character varying NOT NULL,
    commit_id character varying NOT NULL,
    ordinal bigint NOT NULL,
    path text NOT NULL,
    old_content jsonb,
    new_content jsonb,
    visibility character varying NOT NULL,
    PRIMARY KEY (repo_id, commit_id, ordinal)
);

CREATE TABLE scope_visibility_events (
    repo_id character varying NOT NULL,
    id character varying NOT NULL,
    ordinal bigint NOT NULL,
    after_commit_id character varying,
    source_commit_id character varying,
    author_id character varying NOT NULL,
    path text NOT NULL,
    old_visibility character varying NOT NULL,
    new_visibility character varying NOT NULL,
    current_content jsonb,
    PRIMARY KEY (repo_id, id),
    UNIQUE (repo_id, ordinal)
);

CREATE TABLE scope_live_files (
    repo_id character varying NOT NULL,
    path text NOT NULL,
    content jsonb NOT NULL,
    PRIMARY KEY (repo_id, path)
);

CREATE TABLE scope_object_references (
    object_key character varying NOT NULL,
    ref_kind character varying NOT NULL,
    ref_id character varying NOT NULL,
    PRIMARY KEY (object_key, ref_kind, ref_id)
);

CREATE INDEX scope_object_references_owner
    ON scope_object_references (ref_kind, ref_id);


--
-- Name: scope_repository_first_push_tokens; Type: TABLE; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

CREATE TABLE scope_repository_first_push_tokens (
    repo_id character varying NOT NULL,
    token_hash character varying NOT NULL,
    owner_user_id character varying NOT NULL,
    created_at_unix bigint NOT NULL,
    expires_at_unix bigint NOT NULL,
    used_at_unix bigint
);


--
-- Name: scope_repository_git_push_tokens; Type: TABLE; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

CREATE TABLE scope_repository_git_push_tokens (
    repo_id character varying NOT NULL,
    token_hash character varying NOT NULL,
    owner_user_id character varying NOT NULL,
    created_at_unix bigint NOT NULL
);


--
-- Name: scope_git_heads; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE scope_git_heads (
    repo_id character varying NOT NULL,
    head_oid character varying NOT NULL,
    segment_sequence bigint NOT NULL,
    change_version bigint NOT NULL,
    manifest_object_key character varying NOT NULL,
    manifest_sha256 character varying NOT NULL,
    manifest_size_bytes bigint NOT NULL
);

CREATE TABLE scope_git_segments (
    repo_id character varying NOT NULL,
    sequence bigint NOT NULL,
    base_oid character varying,
    head_oid character varying NOT NULL,
    object_key character varying NOT NULL,
    sha256 character varying NOT NULL,
    size_bytes bigint NOT NULL,
    manifest_object_key character varying NOT NULL,
    manifest_sha256 character varying NOT NULL,
    manifest_size_bytes bigint NOT NULL
);


--
-- Name: scope_repository_invites; Type: TABLE; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

CREATE TABLE scope_repository_invites (
    id character varying NOT NULL,
    repo_id character varying NOT NULL,
    invited_email character varying NOT NULL,
    invited_email_normalized character varying NOT NULL,
    permissions jsonb NOT NULL,
    invited_by_user_id character varying NOT NULL,
    state character varying NOT NULL,
    token_hash character varying NOT NULL,
    created_at_unix bigint NOT NULL,
    updated_at_unix bigint NOT NULL,
    expires_at_unix bigint NOT NULL,
    accepted_by_user_id character varying,
    accepted_at_unix bigint,
    revoked_at_unix bigint
);


--
-- Name: scope_repository_members; Type: TABLE; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

CREATE TABLE scope_repository_members (
    repo_id character varying NOT NULL,
    user_id character varying NOT NULL,
    permissions jsonb NOT NULL,
    created_at_unix bigint NOT NULL,
    updated_at_unix bigint NOT NULL
);


--
-- Name: scope_request_events; Type: TABLE; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

CREATE TABLE scope_request_events (
    id character varying NOT NULL,
    request_id character varying NOT NULL,
    actor_user_id character varying NOT NULL,
    kind character varying NOT NULL,
    position bigint NOT NULL,
    payload jsonb NOT NULL,
    created_at_unix bigint NOT NULL
);

CREATE TABLE scope_request_change_blocks (
    id character varying NOT NULL,
    request_id character varying NOT NULL,
    position bigint NOT NULL,
    actor_user_id character varying NOT NULL,
    old_head_oid character varying NOT NULL,
    new_head_oid character varying NOT NULL,
    git_snapshot jsonb NOT NULL,
    created_at_unix bigint NOT NULL
);

CREATE TABLE scope_request_discussions (
    id character varying NOT NULL,
    request_id character varying NOT NULL,
    opened_position bigint NOT NULL,
    last_activity_position bigint NOT NULL,
    author_user_id character varying NOT NULL,
    subject jsonb NOT NULL,
    body_markdown text,
    status character varying NOT NULL,
    client_discussion_id character varying NOT NULL,
    created_at_unix bigint NOT NULL,
    resolved_at_unix bigint,
    resolved_by_user_id character varying
);

CREATE TABLE scope_request_discussion_replies (
    id character varying NOT NULL,
    discussion_id character varying NOT NULL,
    position bigint NOT NULL,
    depth bigint NOT NULL,
    author_user_id character varying NOT NULL,
    body_markdown text NOT NULL,
    reply_to_reply_id character varying,
    client_reply_id character varying NOT NULL,
    created_at_unix bigint NOT NULL
);

CREATE TABLE scope_request_discussion_read_states (
    discussion_id character varying NOT NULL,
    user_id character varying NOT NULL,
    read_through_position bigint NOT NULL,
    updated_at_unix bigint NOT NULL
);

CREATE TABLE scope_request_invitees (
    request_id character varying NOT NULL,
    user_id character varying NOT NULL,
    invited_by_user_id character varying NOT NULL,
    created_at_unix bigint NOT NULL
);


--
-- Name: scope_requests; Type: TABLE; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

CREATE TABLE scope_requests (
    id character varying NOT NULL,
    repo_id character varying NOT NULL,
    name character varying NOT NULL,
    author_user_id character varying NOT NULL,
    author_role character varying NOT NULL,
    audience character varying NOT NULL,
    base_main_oid character varying NOT NULL,
    head_oid character varying NOT NULL,
    git_snapshot jsonb,
    title text NOT NULL,
    description_markdown text NOT NULL,
    state character varying NOT NULL,
    activity_version bigint NOT NULL,
    ready_queue_version bigint,
    current_stake_credits integer NOT NULL,
    first_ready_at_unix bigint,
    ready_at_unix bigint,
    held_at_unix bigint,
    held_by_user_id character varying,
    assessment_outcome character varying,
    assessment_body_markdown text,
    assessed_at_unix bigint,
    assessed_by_user_id character varying,
    completed_at_unix bigint,
    completed_by_user_id character varying,
    merged_at_unix bigint,
    merged_by_user_id character varying,
    merged_head_oid character varying,
    merged_main_oid character varying,
    created_at_unix bigint NOT NULL,
    updated_at_unix bigint NOT NULL
);


--
-- Name: scope_orphan_object_jobs; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE scope_orphan_object_jobs (
    object_key character varying NOT NULL,
    generation character varying NOT NULL,
    sha256 character varying NOT NULL,
    git_oid character varying NOT NULL,
    size_bytes bigint NOT NULL,
    attempts integer NOT NULL,
    next_run_at_unix bigint NOT NULL,
    last_error text,
    completed_at_unix bigint,
    created_at_unix bigint NOT NULL,
    updated_at_unix bigint NOT NULL
);


--
-- Name: scope_user_credit_accounts; Type: TABLE; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

CREATE TABLE scope_user_credit_accounts (
    user_id character varying NOT NULL,
    balance_credits integer NOT NULL
);


--
-- Name: scope_users; Type: TABLE; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

CREATE TABLE scope_users (
    id character varying NOT NULL,
    handle character varying NOT NULL,
    email character varying NOT NULL,
    email_verified boolean NOT NULL
);


--
-- Name: scope_auth_identities pk_scope_auth_identities; Type: CONSTRAINT; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

ALTER TABLE ONLY scope_auth_identities
    ADD CONSTRAINT pk_scope_auth_identities PRIMARY KEY (provider, subject);


--
-- Name: scope_projection_files pk_scope_projection_files; Type: CONSTRAINT; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

ALTER TABLE ONLY scope_projection_files
    ADD CONSTRAINT pk_scope_projection_files PRIMARY KEY (repo_id, source, audience, path_key);


--
-- Name: scope_projection_read_models pk_scope_projection_read_models; Type: CONSTRAINT; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

ALTER TABLE ONLY scope_projection_read_models
    ADD CONSTRAINT pk_scope_projection_read_models PRIMARY KEY (repo_id, source, audience);


--
-- Name: scope_repository_members pk_scope_repository_members; Type: CONSTRAINT; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

ALTER TABLE ONLY scope_repository_members
    ADD CONSTRAINT pk_scope_repository_members PRIMARY KEY (repo_id, user_id);


--
-- Name: scope_cli_browser_logins scope_cli_browser_logins_pkey; Type: CONSTRAINT; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

ALTER TABLE ONLY scope_cli_browser_logins
    ADD CONSTRAINT scope_cli_browser_logins_pkey PRIMARY KEY (request_id);


--
-- Name: scope_cli_device_logins scope_cli_device_logins_pkey; Type: CONSTRAINT; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

ALTER TABLE ONLY scope_cli_device_logins
    ADD CONSTRAINT scope_cli_device_logins_pkey PRIMARY KEY (device_code_hash);


--
-- Name: scope_cli_device_logins scope_cli_device_logins_user_code_hash_key; Type: CONSTRAINT; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

ALTER TABLE ONLY scope_cli_device_logins
    ADD CONSTRAINT scope_cli_device_logins_user_code_hash_key UNIQUE (user_code_hash);


--
-- Name: scope_cli_exchange_grants scope_cli_exchange_grants_pkey; Type: CONSTRAINT; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

ALTER TABLE ONLY scope_cli_exchange_grants
    ADD CONSTRAINT scope_cli_exchange_grants_pkey PRIMARY KEY (grant_hash);


--
-- Name: scope_cli_sessions scope_cli_sessions_pkey; Type: CONSTRAINT; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

ALTER TABLE ONLY scope_cli_sessions
    ADD CONSTRAINT scope_cli_sessions_pkey PRIMARY KEY (id);


--
-- Name: scope_cli_sessions scope_cli_sessions_token_hash_key; Type: CONSTRAINT; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

ALTER TABLE ONLY scope_cli_sessions
    ADD CONSTRAINT scope_cli_sessions_token_hash_key UNIQUE (token_hash);


--
-- Name: scope_credit_ledger_entries scope_credit_ledger_entries_pkey; Type: CONSTRAINT; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

ALTER TABLE ONLY scope_credit_ledger_entries
    ADD CONSTRAINT scope_credit_ledger_entries_pkey PRIMARY KEY (id);


--
-- Name: scope_metadata_locks scope_metadata_locks_pkey; Type: CONSTRAINT; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

ALTER TABLE ONLY scope_metadata_locks
    ADD CONSTRAINT scope_metadata_locks_pkey PRIMARY KEY (key);


--
-- Name: scope_metadata_reset_events scope_metadata_reset_events_pkey; Type: CONSTRAINT; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

ALTER TABLE ONLY scope_metadata_reset_events
    ADD CONSTRAINT scope_metadata_reset_events_pkey PRIMARY KEY (id);


--
-- Name: scope_outbox_jobs scope_outbox_jobs_idempotency_key_key; Type: CONSTRAINT; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

ALTER TABLE ONLY scope_outbox_jobs
    ADD CONSTRAINT scope_outbox_jobs_idempotency_key_key UNIQUE (idempotency_key);


--
-- Name: scope_outbox_jobs scope_outbox_jobs_pkey; Type: CONSTRAINT; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

ALTER TABLE ONLY scope_outbox_jobs
    ADD CONSTRAINT scope_outbox_jobs_pkey PRIMARY KEY (id);


--
-- Name: scope_repo_storage_cleanup_jobs scope_repo_storage_cleanup_jobs_pkey; Type: CONSTRAINT; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

ALTER TABLE ONLY scope_repo_storage_cleanup_jobs
    ADD CONSTRAINT scope_repo_storage_cleanup_jobs_pkey PRIMARY KEY (repo_id);


--
-- Name: scope_repositories scope_repositories_pkey; Type: CONSTRAINT; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

ALTER TABLE ONLY scope_repositories
    ADD CONSTRAINT scope_repositories_pkey PRIMARY KEY (id);


--
-- Name: scope_repository_first_push_tokens scope_repository_first_push_tokens_pkey; Type: CONSTRAINT; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

ALTER TABLE ONLY scope_repository_first_push_tokens
    ADD CONSTRAINT scope_repository_first_push_tokens_pkey PRIMARY KEY (repo_id);


--
-- Name: scope_repository_git_push_tokens scope_repository_git_push_tokens_pkey; Type: CONSTRAINT; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

ALTER TABLE ONLY scope_repository_git_push_tokens
    ADD CONSTRAINT scope_repository_git_push_tokens_pkey PRIMARY KEY (repo_id);


--
-- Name: scope_git_heads scope_git_heads_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY scope_git_heads
    ADD CONSTRAINT scope_git_heads_pkey PRIMARY KEY (repo_id);

ALTER TABLE ONLY scope_git_segments
    ADD CONSTRAINT scope_git_segments_pkey PRIMARY KEY (repo_id, sequence);


--
-- Name: scope_repository_invites scope_repository_invites_pkey; Type: CONSTRAINT; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

ALTER TABLE ONLY scope_repository_invites
    ADD CONSTRAINT scope_repository_invites_pkey PRIMARY KEY (id);


--
-- Name: scope_request_events scope_request_events_pkey; Type: CONSTRAINT; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

ALTER TABLE ONLY scope_request_events
    ADD CONSTRAINT scope_request_events_pkey PRIMARY KEY (id);
ALTER TABLE ONLY scope_request_change_blocks
    ADD CONSTRAINT scope_request_change_blocks_pkey PRIMARY KEY (id);
ALTER TABLE ONLY scope_request_change_blocks
    ADD CONSTRAINT scope_request_change_blocks_position_key UNIQUE (request_id, position);

ALTER TABLE ONLY scope_request_discussions
    ADD CONSTRAINT scope_request_discussions_pkey PRIMARY KEY (id);
ALTER TABLE ONLY scope_request_discussions
    ADD CONSTRAINT scope_request_discussions_position_key UNIQUE (request_id, opened_position);
ALTER TABLE ONLY scope_request_discussions
    ADD CONSTRAINT scope_request_discussions_client_key UNIQUE (request_id, author_user_id, client_discussion_id);
ALTER TABLE ONLY scope_request_discussion_replies
    ADD CONSTRAINT scope_request_discussion_replies_pkey PRIMARY KEY (id);
ALTER TABLE ONLY scope_request_discussion_replies
    ADD CONSTRAINT scope_request_discussion_replies_position_key UNIQUE (discussion_id, position);
ALTER TABLE ONLY scope_request_discussion_replies
    ADD CONSTRAINT scope_request_discussion_replies_client_key UNIQUE (discussion_id, author_user_id, client_reply_id);
ALTER TABLE ONLY scope_request_discussion_read_states
    ADD CONSTRAINT scope_request_discussion_read_states_pkey PRIMARY KEY (discussion_id, user_id);
ALTER TABLE ONLY scope_request_invitees
    ADD CONSTRAINT scope_request_invitees_pkey PRIMARY KEY (request_id, user_id);


--
-- Name: scope_requests scope_requests_pkey; Type: CONSTRAINT; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

ALTER TABLE ONLY scope_requests
    ADD CONSTRAINT scope_requests_pkey PRIMARY KEY (id);


--
-- Name: scope_requests scope_requests_repo_name_key; Type: CONSTRAINT; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

ALTER TABLE ONLY scope_requests
    ADD CONSTRAINT scope_requests_repo_name_key UNIQUE (repo_id, name);


--
-- Name: scope_orphan_object_jobs scope_orphan_object_jobs_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY scope_orphan_object_jobs
    ADD CONSTRAINT scope_orphan_object_jobs_pkey PRIMARY KEY (object_key);


--
-- Name: scope_user_credit_accounts scope_user_credit_accounts_pkey; Type: CONSTRAINT; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

ALTER TABLE ONLY scope_user_credit_accounts
    ADD CONSTRAINT scope_user_credit_accounts_pkey PRIMARY KEY (user_id);


--
-- Name: scope_users scope_users_handle_key; Type: CONSTRAINT; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

ALTER TABLE ONLY scope_users
    ADD CONSTRAINT scope_users_handle_key UNIQUE (handle);


--
-- Name: scope_users scope_users_pkey; Type: CONSTRAINT; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

ALTER TABLE ONLY scope_users
    ADD CONSTRAINT scope_users_pkey PRIMARY KEY (id);


--
-- Name: idx_scope_auth_identities_user; Type: INDEX; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

CREATE INDEX idx_scope_auth_identities_user ON scope_auth_identities USING btree (user_id);


--
-- Name: idx_scope_cli_exchange_grants_user; Type: INDEX; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

CREATE INDEX idx_scope_cli_exchange_grants_user ON scope_cli_exchange_grants USING btree (user_id);


--
-- Name: idx_scope_cli_sessions_user; Type: INDEX; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

CREATE INDEX idx_scope_cli_sessions_user ON scope_cli_sessions USING btree (user_id);


--
-- Name: idx_scope_credit_ledger_entries_request; Type: INDEX; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

CREATE INDEX idx_scope_credit_ledger_entries_request ON scope_credit_ledger_entries USING btree (request_id);


--
-- Name: idx_scope_credit_ledger_entries_user_time; Type: INDEX; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

CREATE INDEX idx_scope_credit_ledger_entries_user_time ON scope_credit_ledger_entries USING btree (user_id, created_at_unix);


--
-- Name: idx_scope_outbox_jobs_ready; Type: INDEX; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

CREATE INDEX idx_scope_outbox_jobs_ready ON scope_outbox_jobs USING btree (state, next_run_at_unix, created_at_unix);


--
-- Name: idx_scope_outbox_jobs_repo; Type: INDEX; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

CREATE INDEX idx_scope_outbox_jobs_repo ON scope_outbox_jobs USING btree (repo_id, repo_version);

CREATE INDEX idx_scope_runner_grants_runner ON scope_runner_grants USING btree (runner_id, revoked_at_unix);
CREATE UNIQUE INDEX idx_scope_runner_grants_active_name ON scope_runner_grants USING btree (repo_id, name)
    WHERE revoked_at_unix IS NULL;
CREATE INDEX idx_scope_runs_queue ON scope_runs USING btree (created_at_unix, id) WHERE state = 'queued';
CREATE INDEX idx_scope_runs_repo ON scope_runs USING btree (repo_id, created_at_unix DESC, id);
CREATE UNIQUE INDEX idx_scope_run_attempts_active ON scope_run_attempts USING btree (run_id)
    WHERE state IN ('leased', 'running');
CREATE INDEX idx_scope_run_attempts_runner ON scope_run_attempts USING btree (runner_id, state);
CREATE INDEX idx_scope_run_attempts_expiring ON scope_run_attempts USING btree (lease_expires_at_unix, id)
    WHERE state IN ('leased', 'running');
CREATE INDEX idx_scope_run_logs_run_position ON scope_run_logs USING btree (run_id, position);
CREATE INDEX idx_scope_run_logs_step_position ON scope_run_logs USING btree (attempt_id, step_index, position);


--
-- Name: idx_scope_projection_files_lookup; Type: INDEX; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

CREATE INDEX idx_scope_projection_files_lookup ON scope_projection_files USING btree (repo_id, repo_version, source, audience);


--
-- Name: idx_scope_repo_storage_cleanup_jobs_pending; Type: INDEX; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

CREATE INDEX idx_scope_repo_storage_cleanup_jobs_pending ON scope_repo_storage_cleanup_jobs USING btree (completed_at_unix, next_run_at_unix);


--
-- Name: idx_scope_repositories_owner_name; Type: INDEX; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

CREATE UNIQUE INDEX idx_scope_repositories_owner_name ON scope_repositories USING btree (owner_handle, name);


--
-- Name: idx_scope_repository_invites_repo_email; Type: INDEX; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

CREATE INDEX idx_scope_repository_invites_repo_email ON scope_repository_invites USING btree (repo_id, invited_email_normalized);


--
-- Name: idx_scope_repository_invites_token_hash; Type: INDEX; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

CREATE INDEX idx_scope_repository_invites_token_hash ON scope_repository_invites USING btree (token_hash);


--
-- Name: idx_scope_repository_members_user; Type: INDEX; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

CREATE INDEX idx_scope_repository_members_user ON scope_repository_members USING btree (user_id);


--
-- Name: idx_scope_request_events_request_time; Type: INDEX; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

CREATE UNIQUE INDEX idx_scope_request_events_request_position ON scope_request_events USING btree (request_id, position);
CREATE INDEX idx_scope_request_change_blocks_request_position ON scope_request_change_blocks USING btree (request_id, position DESC, id);
CREATE INDEX idx_scope_request_discussions_newest ON scope_request_discussions USING btree (request_id, opened_position DESC, id);
CREATE INDEX idx_scope_request_discussion_replies_position ON scope_request_discussion_replies USING btree (discussion_id, position DESC, id);
CREATE INDEX idx_scope_request_discussion_replies_tree ON scope_request_discussion_replies USING btree (discussion_id, reply_to_reply_id, position DESC, id);
CREATE INDEX idx_scope_request_discussion_replies_parent ON scope_request_discussion_replies USING btree (reply_to_reply_id, position DESC, id) WHERE reply_to_reply_id IS NOT NULL;
CREATE INDEX idx_scope_request_invitees_user ON scope_request_invitees USING btree (user_id, request_id);


--
-- Name: idx_scope_requests_author; Type: INDEX; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

CREATE INDEX idx_scope_requests_author ON scope_requests USING btree (author_user_id);

CREATE INDEX idx_scope_requests_repo_id ON scope_requests USING btree (repo_id, id);

CREATE INDEX idx_scope_requests_repo_audience_id ON scope_requests USING btree (repo_id, audience, id);


--
-- Name: idx_scope_requests_repo_state; Type: INDEX; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

CREATE INDEX idx_scope_requests_repo_state ON scope_requests USING btree (repo_id, state);
CREATE INDEX idx_scope_requests_ready_queue ON scope_requests USING btree (
    repo_id,
    ready_queue_version,
    current_stake_credits DESC,
    ready_at_unix,
    id
) WHERE state = 'ReadyForReview';
CREATE INDEX idx_scope_requests_completed ON scope_requests USING btree (
    repo_id,
    completed_at_unix DESC,
    id
) WHERE state = 'Completed';


--
-- Name: idx_scope_orphan_object_jobs_pending; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX idx_scope_orphan_object_jobs_pending ON scope_orphan_object_jobs USING btree (completed_at_unix, next_run_at_unix);


--
-- Name: idx_scope_users_email; Type: INDEX; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

CREATE UNIQUE INDEX idx_scope_users_email ON scope_users USING btree (email);


--
-- Name: scope_auth_identities fk_scope_auth_identities_user; Type: FK CONSTRAINT; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

ALTER TABLE ONLY scope_auth_identities
    ADD CONSTRAINT fk_scope_auth_identities_user FOREIGN KEY (user_id) REFERENCES scope_users(id) ON DELETE CASCADE;


--
-- Name: scope_cli_browser_logins fk_scope_cli_browser_logins_completed_user; Type: FK CONSTRAINT; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

ALTER TABLE ONLY scope_cli_browser_logins
    ADD CONSTRAINT fk_scope_cli_browser_logins_completed_user FOREIGN KEY (completed_user_id) REFERENCES scope_users(id) ON DELETE CASCADE;


--
-- Name: scope_cli_device_logins fk_scope_cli_device_logins_completed_user; Type: FK CONSTRAINT; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

ALTER TABLE ONLY scope_cli_device_logins
    ADD CONSTRAINT fk_scope_cli_device_logins_completed_user FOREIGN KEY (completed_user_id) REFERENCES scope_users(id) ON DELETE CASCADE;


--
-- Name: scope_cli_exchange_grants fk_scope_cli_exchange_grants_user; Type: FK CONSTRAINT; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

ALTER TABLE ONLY scope_cli_exchange_grants
    ADD CONSTRAINT fk_scope_cli_exchange_grants_user FOREIGN KEY (user_id) REFERENCES scope_users(id) ON DELETE CASCADE;


--
-- Name: scope_cli_sessions fk_scope_cli_sessions_user; Type: FK CONSTRAINT; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

ALTER TABLE ONLY scope_cli_sessions
    ADD CONSTRAINT fk_scope_cli_sessions_user FOREIGN KEY (user_id) REFERENCES scope_users(id) ON DELETE CASCADE;


--
-- Name: scope_credit_ledger_entries fk_scope_credit_ledger_entries_request; Type: FK CONSTRAINT; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

ALTER TABLE ONLY scope_credit_ledger_entries
    ADD CONSTRAINT fk_scope_credit_ledger_entries_request FOREIGN KEY (request_id) REFERENCES scope_requests(id) ON DELETE SET NULL;


--
-- Name: scope_credit_ledger_entries fk_scope_credit_ledger_entries_user; Type: FK CONSTRAINT; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

ALTER TABLE ONLY scope_credit_ledger_entries
    ADD CONSTRAINT fk_scope_credit_ledger_entries_user FOREIGN KEY (user_id) REFERENCES scope_users(id) ON DELETE CASCADE;


--
-- Name: scope_outbox_jobs fk_scope_outbox_jobs_repo; Type: FK CONSTRAINT; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

ALTER TABLE ONLY scope_outbox_jobs
    ADD CONSTRAINT fk_scope_outbox_jobs_repo FOREIGN KEY (repo_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_runners
    ADD CONSTRAINT fk_scope_runners_owner FOREIGN KEY (owner_user_id) REFERENCES scope_users(id) ON DELETE RESTRICT;
ALTER TABLE ONLY scope_runner_grants
    ADD CONSTRAINT fk_scope_runner_grants_repo FOREIGN KEY (repo_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;
ALTER TABLE ONLY scope_runner_grants
    ADD CONSTRAINT fk_scope_runner_grants_runner FOREIGN KEY (runner_id) REFERENCES scope_runners(id) ON DELETE RESTRICT;
ALTER TABLE ONLY scope_runner_grants
    ADD CONSTRAINT fk_scope_runner_grants_actor FOREIGN KEY (granted_by_user_id) REFERENCES scope_users(id) ON DELETE RESTRICT;
ALTER TABLE ONLY scope_runs
    ADD CONSTRAINT fk_scope_runs_repo FOREIGN KEY (repo_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;
ALTER TABLE ONLY scope_push_trigger_evaluations
    ADD CONSTRAINT fk_scope_push_trigger_evaluations_repo FOREIGN KEY (repo_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;
ALTER TABLE ONLY scope_runs
    ADD CONSTRAINT fk_scope_runs_revision FOREIGN KEY (workflow_revision_digest) REFERENCES scope_workflow_revisions(digest) ON DELETE RESTRICT;
ALTER TABLE ONLY scope_runs
    ADD CONSTRAINT fk_scope_runs_requester FOREIGN KEY (requested_by_user_id) REFERENCES scope_users(id) ON DELETE RESTRICT;
ALTER TABLE ONLY scope_run_attempts
    ADD CONSTRAINT fk_scope_run_attempts_run FOREIGN KEY (run_id) REFERENCES scope_runs(id) ON DELETE CASCADE;
ALTER TABLE ONLY scope_run_attempts
    ADD CONSTRAINT fk_scope_run_attempts_runner FOREIGN KEY (runner_id) REFERENCES scope_runners(id) ON DELETE RESTRICT;
ALTER TABLE ONLY scope_run_attempt_steps
    ADD CONSTRAINT fk_scope_run_attempt_steps_attempt FOREIGN KEY (attempt_id) REFERENCES scope_run_attempts(id) ON DELETE CASCADE;
ALTER TABLE ONLY scope_run_logs
    ADD CONSTRAINT fk_scope_run_logs_run FOREIGN KEY (run_id) REFERENCES scope_runs(id) ON DELETE CASCADE;
ALTER TABLE ONLY scope_run_logs
    ADD CONSTRAINT fk_scope_run_logs_step FOREIGN KEY (attempt_id, step_index) REFERENCES scope_run_attempt_steps(attempt_id, step_index) ON DELETE CASCADE;
ALTER TABLE ONLY scope_runs
    ADD CONSTRAINT fk_scope_runs_current_attempt FOREIGN KEY (current_attempt_id) REFERENCES scope_run_attempts(id) ON DELETE SET NULL;


--
-- Name: scope_projection_files fk_scope_projection_files_repo; Type: FK CONSTRAINT; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

ALTER TABLE ONLY scope_projection_files
    ADD CONSTRAINT fk_scope_projection_files_repo FOREIGN KEY (repo_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;


--
-- Name: scope_projection_read_models fk_scope_projection_read_models_repo; Type: FK CONSTRAINT; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

ALTER TABLE ONLY scope_projection_read_models
    ADD CONSTRAINT fk_scope_projection_read_models_repo FOREIGN KEY (repo_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;


--
-- Name: scope_repositories fk_scope_repositories_owner; Type: FK CONSTRAINT; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

ALTER TABLE ONLY scope_repositories
    ADD CONSTRAINT fk_scope_repositories_owner FOREIGN KEY (owner_user_id) REFERENCES scope_users(id) ON DELETE CASCADE;


--
-- Name: scope_repository_first_push_tokens fk_scope_repository_first_push_tokens_owner; Type: FK CONSTRAINT; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

ALTER TABLE ONLY scope_repository_first_push_tokens
    ADD CONSTRAINT fk_scope_repository_first_push_tokens_owner FOREIGN KEY (owner_user_id) REFERENCES scope_users(id) ON DELETE CASCADE;


--
-- Name: scope_repository_first_push_tokens fk_scope_repository_first_push_tokens_repo; Type: FK CONSTRAINT; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

ALTER TABLE ONLY scope_repository_first_push_tokens
    ADD CONSTRAINT fk_scope_repository_first_push_tokens_repo FOREIGN KEY (repo_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;


--
-- Name: scope_repository_git_push_tokens fk_scope_repository_git_push_tokens_owner; Type: FK CONSTRAINT; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

ALTER TABLE ONLY scope_repository_git_push_tokens
    ADD CONSTRAINT fk_scope_repository_git_push_tokens_owner FOREIGN KEY (owner_user_id) REFERENCES scope_users(id) ON DELETE CASCADE;


--
-- Name: scope_repository_git_push_tokens fk_scope_repository_git_push_tokens_repo; Type: FK CONSTRAINT; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

ALTER TABLE ONLY scope_repository_git_push_tokens
    ADD CONSTRAINT fk_scope_repository_git_push_tokens_repo FOREIGN KEY (repo_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;


--
-- Name: scope_git_heads fk_scope_git_heads_repo; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY scope_git_heads
    ADD CONSTRAINT fk_scope_git_heads_repo FOREIGN KEY (repo_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_git_segments
    ADD CONSTRAINT fk_scope_git_segments_repo FOREIGN KEY (repo_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;


--
-- Name: scope_repository_invites fk_scope_repository_invites_accepted_user; Type: FK CONSTRAINT; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

ALTER TABLE ONLY scope_repository_invites
    ADD CONSTRAINT fk_scope_repository_invites_accepted_user FOREIGN KEY (accepted_by_user_id) REFERENCES scope_users(id) ON DELETE SET NULL;


--
-- Name: scope_repository_invites fk_scope_repository_invites_inviter; Type: FK CONSTRAINT; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

ALTER TABLE ONLY scope_repository_invites
    ADD CONSTRAINT fk_scope_repository_invites_inviter FOREIGN KEY (invited_by_user_id) REFERENCES scope_users(id) ON DELETE CASCADE;


--
-- Name: scope_repository_invites fk_scope_repository_invites_repo; Type: FK CONSTRAINT; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

ALTER TABLE ONLY scope_repository_invites
    ADD CONSTRAINT fk_scope_repository_invites_repo FOREIGN KEY (repo_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;


--
-- Name: scope_repository_members fk_scope_repository_members_repo; Type: FK CONSTRAINT; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

ALTER TABLE ONLY scope_repository_members
    ADD CONSTRAINT fk_scope_repository_members_repo FOREIGN KEY (repo_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;


--
-- Name: scope_repository_members fk_scope_repository_members_user; Type: FK CONSTRAINT; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

ALTER TABLE ONLY scope_repository_members
    ADD CONSTRAINT fk_scope_repository_members_user FOREIGN KEY (user_id) REFERENCES scope_users(id) ON DELETE CASCADE;


--
-- Name: scope_request_events fk_scope_request_events_actor; Type: FK CONSTRAINT; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

ALTER TABLE ONLY scope_request_events
    ADD CONSTRAINT fk_scope_request_events_actor FOREIGN KEY (actor_user_id) REFERENCES scope_users(id) ON DELETE CASCADE;


--
-- Name: scope_request_events fk_scope_request_events_request; Type: FK CONSTRAINT; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

ALTER TABLE ONLY scope_request_events
    ADD CONSTRAINT fk_scope_request_events_request FOREIGN KEY (request_id) REFERENCES scope_requests(id) ON DELETE CASCADE;
ALTER TABLE ONLY scope_request_change_blocks
    ADD CONSTRAINT fk_scope_request_change_blocks_request FOREIGN KEY (request_id) REFERENCES scope_requests(id) ON DELETE CASCADE;
ALTER TABLE ONLY scope_request_change_blocks
    ADD CONSTRAINT fk_scope_request_change_blocks_actor FOREIGN KEY (actor_user_id) REFERENCES scope_users(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_request_discussions
    ADD CONSTRAINT fk_scope_request_discussions_request FOREIGN KEY (request_id) REFERENCES scope_requests(id) ON DELETE CASCADE;
ALTER TABLE ONLY scope_request_discussions
    ADD CONSTRAINT fk_scope_request_discussions_author FOREIGN KEY (author_user_id) REFERENCES scope_users(id) ON DELETE CASCADE;
ALTER TABLE ONLY scope_request_discussions
    ADD CONSTRAINT fk_scope_request_discussions_resolver FOREIGN KEY (resolved_by_user_id) REFERENCES scope_users(id) ON DELETE SET NULL;
ALTER TABLE ONLY scope_request_discussion_replies
    ADD CONSTRAINT fk_scope_request_discussion_replies_discussion FOREIGN KEY (discussion_id) REFERENCES scope_request_discussions(id) ON DELETE CASCADE;
ALTER TABLE ONLY scope_request_discussion_replies
    ADD CONSTRAINT fk_scope_request_discussion_replies_author FOREIGN KEY (author_user_id) REFERENCES scope_users(id) ON DELETE CASCADE;
ALTER TABLE ONLY scope_request_discussion_replies
    ADD CONSTRAINT fk_scope_request_discussion_replies_quoted_reply FOREIGN KEY (reply_to_reply_id) REFERENCES scope_request_discussion_replies(id) ON DELETE RESTRICT;
ALTER TABLE ONLY scope_request_discussion_read_states
    ADD CONSTRAINT fk_scope_request_discussion_read_states_discussion FOREIGN KEY (discussion_id) REFERENCES scope_request_discussions(id) ON DELETE CASCADE;
ALTER TABLE ONLY scope_request_discussion_read_states
    ADD CONSTRAINT fk_scope_request_discussion_read_states_user FOREIGN KEY (user_id) REFERENCES scope_users(id) ON DELETE CASCADE;
ALTER TABLE ONLY scope_request_invitees
    ADD CONSTRAINT fk_scope_request_invitees_request FOREIGN KEY (request_id) REFERENCES scope_requests(id) ON DELETE CASCADE;
ALTER TABLE ONLY scope_request_invitees
    ADD CONSTRAINT fk_scope_request_invitees_user FOREIGN KEY (user_id) REFERENCES scope_users(id) ON DELETE CASCADE;
ALTER TABLE ONLY scope_request_invitees
    ADD CONSTRAINT fk_scope_request_invitees_inviter FOREIGN KEY (invited_by_user_id) REFERENCES scope_users(id) ON DELETE RESTRICT;


--
-- Name: scope_requests fk_scope_requests_author; Type: FK CONSTRAINT; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

ALTER TABLE ONLY scope_requests
    ADD CONSTRAINT fk_scope_requests_author FOREIGN KEY (author_user_id) REFERENCES scope_users(id) ON DELETE CASCADE;


--
-- Name: scope_requests fk_scope_requests_repo; Type: FK CONSTRAINT; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

ALTER TABLE ONLY scope_requests
    ADD CONSTRAINT fk_scope_requests_repo FOREIGN KEY (repo_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;
ALTER TABLE ONLY scope_requests
    ADD CONSTRAINT fk_scope_requests_holder FOREIGN KEY (held_by_user_id) REFERENCES scope_users(id) ON DELETE RESTRICT;
ALTER TABLE ONLY scope_requests
    ADD CONSTRAINT fk_scope_requests_assessor FOREIGN KEY (assessed_by_user_id) REFERENCES scope_users(id) ON DELETE RESTRICT;
ALTER TABLE ONLY scope_requests
    ADD CONSTRAINT fk_scope_requests_completer FOREIGN KEY (completed_by_user_id) REFERENCES scope_users(id) ON DELETE RESTRICT;
ALTER TABLE ONLY scope_requests
    ADD CONSTRAINT fk_scope_requests_merger FOREIGN KEY (merged_by_user_id) REFERENCES scope_users(id) ON DELETE RESTRICT;


--
-- Name: scope_user_credit_accounts fk_scope_user_credit_accounts_user; Type: FK CONSTRAINT; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

ALTER TABLE ONLY scope_user_credit_accounts
    ADD CONSTRAINT fk_scope_user_credit_accounts_user FOREIGN KEY (user_id) REFERENCES scope_users(id) ON DELETE CASCADE;


--
-- Domain/persistence boundary constraints. The application converts these
-- values fallibly as well; constraints keep invalid rows from entering through
-- operator SQL or future adapters.
ALTER TABLE scope_repositories
    ADD CONSTRAINT scope_repositories_nonnegative_version CHECK (change_version >= 0);
ALTER TABLE scope_repository_first_push_tokens
    ADD CONSTRAINT scope_first_push_token_times CHECK (
        created_at_unix >= 0 AND expires_at_unix >= 0 AND
        (used_at_unix IS NULL OR used_at_unix >= 0)
    );
ALTER TABLE scope_repository_git_push_tokens
    ADD CONSTRAINT scope_git_push_token_time CHECK (created_at_unix >= 0);
ALTER TABLE scope_git_heads
    ADD CONSTRAINT scope_git_head_values CHECK (
        segment_sequence >= 0 AND change_version >= 0 AND manifest_size_bytes >= 0
    );
ALTER TABLE scope_git_segments
    ADD CONSTRAINT scope_git_segment_values CHECK (
        sequence > 0 AND size_bytes >= 0 AND manifest_size_bytes >= 0
    );
ALTER TABLE scope_runners
    ADD CONSTRAINT scope_runners_values CHECK (
        char_length(secret_hash) = 64 AND secret_hash ~ '^[0-9A-Fa-f]+$' AND
        char_length(version) BETWEEN 1 AND 100 AND protocol_version >= 0 AND
        created_at_unix >= 0 AND
        (last_seen_at_unix IS NULL OR last_seen_at_unix >= created_at_unix)
    );
ALTER TABLE scope_runner_grants
    ADD CONSTRAINT scope_runner_grants_values CHECK (
        char_length(name) BETWEEN 1 AND 64 AND name <> 'any' AND
        name ~ '^[A-Za-z0-9][A-Za-z0-9._-]*$' AND
        created_at_unix >= 0 AND (revoked_at_unix IS NULL OR revoked_at_unix >= created_at_unix)
    );
ALTER TABLE scope_workflow_revisions
    ADD CONSTRAINT scope_workflow_revisions_values CHECK (
        char_length(digest) = 64 AND digest ~ '^[0-9A-Fa-f]+$' AND created_at_unix >= 0
    );
ALTER TABLE scope_push_trigger_evaluations
    ADD CONSTRAINT scope_push_trigger_evaluation_values CHECK (
        change_version > 0 AND length(head_oid) = 40 AND created_at_unix >= 0 AND
        state IN ('pending', 'succeeded', 'configuration-error', 'failed') AND
        (
            (state = 'pending' AND message IS NULL AND completed_at_unix IS NULL) OR
            (state = 'succeeded' AND message IS NULL AND completed_at_unix IS NOT NULL) OR
            (state IN ('configuration-error', 'failed') AND
             length(btrim(message)) > 0 AND completed_at_unix IS NOT NULL)
        )
    );
ALTER TABLE scope_runs
    ADD CONSTRAINT scope_runs_values CHECK (
        char_length(workflow_revision_digest) = 64 AND workflow_revision_digest ~ '^[0-9A-Fa-f]+$' AND
        (
            (
                source->>'kind' = 'ephemeral-git-bundle' AND
                char_length(source#>>'{object,sha256}') = 64 AND
                (source#>>'{object,sha256}') ~ '^[0-9A-Fa-f]+$' AND
                char_length(source#>>'{object,git_oid}') = 40 AND
                (source#>>'{object,git_oid}') ~ '^[0-9A-Fa-f]+$'
            ) OR (
                source->>'kind' = 'accepted-revision' AND
                (source->>'change_version')::numeric > 0 AND
                source->>'audience' IN ('Private', 'Public') AND
                char_length(source#>>'{manifest,sha256}') = 64 AND
                (source#>>'{manifest,sha256}') ~ '^[0-9A-Fa-f]+$' AND
                char_length(source#>>'{snapshot,sha256}') = 64 AND
                (source#>>'{snapshot,sha256}') ~ '^[0-9A-Fa-f]+$' AND
                char_length(source#>>'{snapshot,git_oid}') = 40 AND
                (source#>>'{snapshot,git_oid}') ~ '^[0-9A-Fa-f]+$' AND
                (source#>>'{manifest,git_oid}') = (source#>>'{snapshot,git_oid}')
            )
        ) AND
        (
            pinned_container_image IS NULL OR
            pinned_container_image ~ '^[^@[:space:]]+@sha256:[0-9A-Fa-f]{64}$'
        ) AND
        trigger IN ('manual', 'push-main') AND
        state IN ('queued', 'leased', 'running', 'succeeded', 'failed', 'canceled', 'lost') AND
        last_attempt_number >= 0 AND created_at_unix >= 0 AND updated_at_unix >= created_at_unix AND
        (state NOT IN ('leased', 'running') OR current_attempt_id IS NOT NULL) AND
        (state <> 'queued' OR (NOT cancellation_requested AND current_attempt_id IS NULL)) AND
        ((state IN ('succeeded', 'failed', 'canceled', 'lost')) = (completed_at_unix IS NOT NULL)) AND
        (completed_at_unix IS NULL OR completed_at_unix = updated_at_unix) AND
        (state <> 'canceled' OR cancellation_requested) AND
        (trigger <> 'manual' OR requested_by_user_id IS NOT NULL)
    );
ALTER TABLE scope_run_attempts
    ADD CONSTRAINT scope_run_attempts_values CHECK (
        number > 0 AND char_length(runner_name) BETWEEN 1 AND 64 AND
        char_length(token_hash) = 64 AND token_hash ~ '^[0-9A-Fa-f]+$' AND
        state IN ('leased', 'running', 'succeeded', 'failed', 'canceled', 'lost') AND
        token_expires_at_unix = lease_expires_at_unix AND
        created_at_unix >= 0 AND
        last_heartbeat_at_unix >= created_at_unix AND
        last_heartbeat_at_unix < lease_expires_at_unix AND
        (started_at_unix IS NULL OR
            (started_at_unix >= created_at_unix AND started_at_unix < lease_expires_at_unix)) AND
        (completed_at_unix IS NULL OR completed_at_unix >= last_heartbeat_at_unix) AND
        (started_at_unix IS NULL OR completed_at_unix IS NULL OR completed_at_unix >= started_at_unix) AND
        log_bytes >= 0 AND log_bytes <= 10485760 AND
        ((state IN ('succeeded', 'failed', 'canceled', 'lost')) = (completed_at_unix IS NOT NULL)) AND
        (state <> 'succeeded' OR (started_at_unix IS NOT NULL AND terminal_reason IS NULL)) AND
        (state NOT IN ('failed', 'canceled', 'lost') OR terminal_reason IS NOT NULL) AND
        (state IN ('failed', 'canceled', 'lost') OR terminal_reason IS NULL)
    );
ALTER TABLE scope_run_attempt_steps
    ADD CONSTRAINT scope_run_attempt_steps_values CHECK (
        step_index >= 0 AND
        state IN ('pending', 'running', 'succeeded', 'failed', 'canceled', 'lost', 'skipped') AND
        ((state IN ('succeeded', 'failed', 'canceled', 'lost', 'skipped')) = (completed_at_unix IS NOT NULL)) AND
        (started_at_unix IS NULL OR completed_at_unix IS NULL OR completed_at_unix >= started_at_unix) AND
        (state <> 'pending' OR (started_at_unix IS NULL AND completed_at_unix IS NULL AND exit_code IS NULL)) AND
        (state <> 'running' OR (started_at_unix IS NOT NULL AND completed_at_unix IS NULL AND exit_code IS NULL)) AND
        (state <> 'succeeded' OR (started_at_unix IS NOT NULL AND exit_code = 0)) AND
        (state <> 'failed' OR (started_at_unix IS NOT NULL AND exit_code IS NOT NULL AND exit_code <> 0)) AND
        (state IN ('failed', 'succeeded') OR exit_code IS NULL) AND
        (state <> 'skipped' OR started_at_unix IS NULL) AND
        (state NOT IN ('canceled', 'lost') OR started_at_unix IS NOT NULL)
    );
ALTER TABLE scope_run_logs
    ADD CONSTRAINT scope_run_logs_values CHECK (
        position > 0 AND sequence > 0 AND octet_length(text) BETWEEN 1 AND 65536 AND created_at_unix >= 0
    );
ALTER TABLE scope_repository_members
    ADD CONSTRAINT scope_repository_member_times CHECK (
        created_at_unix >= 0 AND updated_at_unix >= 0
    );
ALTER TABLE scope_repository_invites
    ADD CONSTRAINT scope_repository_invite_times CHECK (
        created_at_unix >= 0 AND updated_at_unix >= 0 AND expires_at_unix >= 0 AND
        (accepted_at_unix IS NULL OR accepted_at_unix >= 0) AND
        (revoked_at_unix IS NULL OR revoked_at_unix >= 0)
    );
ALTER TABLE scope_requests
    ADD CONSTRAINT scope_request_nonnegative_values CHECK (
        current_stake_credits >= 0 AND current_stake_credits <= 25 AND
        activity_version >= 0 AND
        (ready_queue_version IS NULL OR ready_queue_version > 0) AND
        (first_ready_at_unix IS NULL) = (ready_queue_version IS NULL) AND
        created_at_unix >= 0 AND updated_at_unix >= created_at_unix AND
        (first_ready_at_unix IS NULL OR first_ready_at_unix BETWEEN created_at_unix AND updated_at_unix) AND
        (ready_at_unix IS NULL OR ready_at_unix BETWEEN created_at_unix AND updated_at_unix) AND
        (held_at_unix IS NULL OR held_at_unix BETWEEN created_at_unix AND updated_at_unix) AND
        (assessed_at_unix IS NULL OR assessed_at_unix BETWEEN created_at_unix AND updated_at_unix) AND
        (completed_at_unix IS NULL OR completed_at_unix BETWEEN created_at_unix AND updated_at_unix) AND
        (merged_at_unix IS NULL OR merged_at_unix BETWEEN created_at_unix AND updated_at_unix)
    ),
    ADD CONSTRAINT scope_request_identity_values CHECK (
        name ~ '^[a-z0-9][a-z0-9-]{0,47}$' AND
        name NOT IN ('main', 'head', 'scope') AND
        audience IN ('Public', 'Private') AND
        author_role IN ('Public', 'Member', 'Owner')
    ),
    ADD CONSTRAINT scope_request_lifecycle_values CHECK (
        state IN ('Working', 'ReadyForReview', 'Completed') AND
        (
            (state = 'Working' AND ready_at_unix IS NULL AND current_stake_credits = 0 AND
             held_at_unix IS NULL AND held_by_user_id IS NULL) OR
            (state = 'ReadyForReview' AND first_ready_at_unix IS NOT NULL AND
             ready_at_unix IS NOT NULL AND ready_at_unix >= first_ready_at_unix AND
             (
                 (audience = 'Public' AND author_role = 'Public' AND current_stake_credits > 0) OR
                 ((audience <> 'Public' OR author_role <> 'Public') AND current_stake_credits = 0)
             )) OR
            (state = 'Completed' AND first_ready_at_unix IS NOT NULL AND
             ready_at_unix IS NULL AND current_stake_credits = 0 AND
             held_at_unix IS NULL AND held_by_user_id IS NULL AND
             completed_at_unix >= first_ready_at_unix)
        ) AND
        (
            (held_at_unix IS NULL AND held_by_user_id IS NULL) OR
            (held_at_unix IS NOT NULL AND held_by_user_id IS NOT NULL AND
             state = 'ReadyForReview' AND held_at_unix >= ready_at_unix)
        )
    ),
    ADD CONSTRAINT scope_request_completion_coherence CHECK (
        (
            state <> 'Completed' AND completed_at_unix IS NULL AND completed_by_user_id IS NULL
        ) OR (
            state = 'Completed' AND completed_at_unix IS NOT NULL AND completed_by_user_id IS NOT NULL
        )
    ),
    ADD CONSTRAINT scope_request_assessment_values CHECK (
        assessment_outcome IS NULL OR assessment_outcome IN ('Accepted', 'Neutral', 'Rejected')
    ),
    ADD CONSTRAINT scope_request_assessment_coherence CHECK (
        (
            assessment_outcome IS NULL AND assessment_body_markdown IS NULL AND
            assessed_at_unix IS NULL AND assessed_by_user_id IS NULL
        ) OR (
            assessment_outcome IS NOT NULL AND assessed_at_unix IS NOT NULL AND
            assessed_by_user_id IS NOT NULL AND state = 'Completed' AND
            assessed_at_unix = completed_at_unix AND
            (
                assessment_outcome <> 'Rejected' OR
                (assessment_body_markdown IS NOT NULL AND length(btrim(assessment_body_markdown)) > 0)
            )
        )
    ),
    ADD CONSTRAINT scope_request_merge_coherence CHECK (
        (
            merged_at_unix IS NULL AND merged_by_user_id IS NULL AND
            merged_head_oid IS NULL AND merged_main_oid IS NULL
        ) OR (
            merged_at_unix IS NOT NULL AND merged_by_user_id IS NOT NULL AND
            merged_head_oid IS NOT NULL AND length(merged_head_oid) > 0 AND
            merged_main_oid IS NOT NULL AND length(merged_main_oid) > 0 AND
            state = 'Completed' AND assessment_outcome = 'Accepted' AND
            merged_at_unix >= completed_at_unix
        )
    );
ALTER TABLE scope_request_events
    ADD CONSTRAINT scope_request_event_values CHECK (position > 0 AND created_at_unix >= 0);
ALTER TABLE scope_request_change_blocks
    ADD CONSTRAINT scope_request_change_block_values CHECK (
        position > 0 AND created_at_unix >= 0 AND
        length(old_head_oid) > 0 AND length(new_head_oid) > 0
    );
ALTER TABLE scope_request_discussions
    ADD CONSTRAINT scope_request_discussion_values CHECK (
        opened_position > 0 AND last_activity_position >= opened_position AND
        status IN ('Dormant', 'Open', 'Resolved') AND
        ((subject ? 'Comment' AND length(btrim(body_markdown)) > 0) OR
         (subject ? 'ChangeBlock' AND body_markdown IS NULL)) AND
        created_at_unix >= 0 AND (resolved_at_unix IS NULL OR resolved_at_unix >= 0)
    );
ALTER TABLE scope_request_discussion_replies
    ADD CONSTRAINT scope_request_discussion_reply_values CHECK (
        position > 0 AND depth >= 0 AND depth <= 16 AND
        length(btrim(body_markdown)) > 0 AND created_at_unix >= 0
    );
ALTER TABLE scope_request_discussion_read_states
    ADD CONSTRAINT scope_request_discussion_read_values CHECK (
        read_through_position >= 0 AND updated_at_unix >= 0
    );
ALTER TABLE scope_request_invitees
    ADD CONSTRAINT scope_request_invitee_values CHECK (created_at_unix >= 0);
ALTER TABLE scope_user_credit_accounts
    ADD CONSTRAINT scope_user_credit_balance CHECK (balance_credits >= 0);
ALTER TABLE scope_credit_ledger_entries
    ADD CONSTRAINT scope_credit_ledger_entry_time CHECK (created_at_unix >= 0);
ALTER TABLE scope_credit_ledger_entries
    ADD CONSTRAINT scope_credit_ledger_entry_values CHECK (
        amount_credits <> 0 AND
        (
            (kind = 'StarterGrant' AND amount_credits > 0 AND request_id IS NULL) OR
            (kind = 'ReviewStakeDebit' AND amount_credits < 0) OR
            (kind IN ('ReviewStakeRefund', 'AssessmentReward') AND amount_credits > 0) OR
            kind = 'AdminAdjustment'
        )
    );
ALTER TABLE scope_projection_read_models
    ADD CONSTRAINT scope_projection_read_model_values CHECK (
        repo_version >= 0 AND rebuilt_at_unix >= 0 AND file_count >= 0 AND
        source = 'live' AND audience IN ('private', 'public')
    );
ALTER TABLE scope_projection_files
    ADD CONSTRAINT scope_projection_file_values CHECK (
        repo_version >= 0 AND source = 'live' AND audience IN ('private', 'public') AND
        size_bytes >= 0 AND git_file_mode IN ('100644', '100755')
    );
ALTER TABLE scope_repo_storage_cleanup_jobs
    ADD CONSTRAINT scope_repo_cleanup_values CHECK (
        attempts >= 0 AND next_run_at_unix >= 0 AND created_at_unix >= 0 AND
        updated_at_unix >= 0 AND (completed_at_unix IS NULL OR completed_at_unix >= 0)
    );
ALTER TABLE scope_orphan_object_jobs
    ADD CONSTRAINT scope_blob_cleanup_values CHECK (
        size_bytes >= 0 AND attempts >= 0 AND next_run_at_unix >= 0 AND
        created_at_unix >= 0 AND updated_at_unix >= 0 AND
        (completed_at_unix IS NULL OR completed_at_unix >= 0)
    );
ALTER TABLE scope_outbox_jobs
    ADD CONSTRAINT scope_outbox_values CHECK (
        repo_version >= 0 AND attempts >= 0 AND next_run_at_unix >= 0 AND
        created_at_unix >= 0 AND updated_at_unix >= 0 AND
        state IN ('ready', 'running', 'succeeded', 'failed') AND
        (lease_expires_at_unix IS NULL OR lease_expires_at_unix >= 0) AND
        (completed_at_unix IS NULL OR completed_at_unix >= 0)
    );
ALTER TABLE scope_metadata_reset_events
    ADD CONSTRAINT scope_metadata_reset_event_time CHECK (reset_at_unix >= 0);
ALTER TABLE scope_logical_commits
    ADD CONSTRAINT fk_scope_logical_commits_repo FOREIGN KEY (repo_id) REFERENCES scope_repositories(id) ON DELETE CASCADE,
    ADD CONSTRAINT scope_logical_commit_ordinal CHECK (ordinal >= 0);
ALTER TABLE scope_file_changes
    ADD CONSTRAINT fk_scope_file_changes_commit FOREIGN KEY (repo_id, commit_id) REFERENCES scope_logical_commits(repo_id, id) ON DELETE CASCADE,
    ADD CONSTRAINT scope_file_change_ordinal CHECK (ordinal >= 0);
ALTER TABLE scope_visibility_events
    ADD CONSTRAINT fk_scope_visibility_events_repo FOREIGN KEY (repo_id) REFERENCES scope_repositories(id) ON DELETE CASCADE,
    ADD CONSTRAINT scope_visibility_event_ordinal CHECK (ordinal >= 0);
ALTER TABLE scope_live_files
    ADD CONSTRAINT fk_scope_live_files_repo FOREIGN KEY (repo_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;

-- PostgreSQL database dump complete
--
