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
    parent_ids jsonb NOT NULL,
    author_id character varying NOT NULL,
    author_visibility character varying NOT NULL,
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
    body text,
    old_head_oid character varying,
    new_head_oid character varying,
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
    state character varying NOT NULL,
    stake_credits integer NOT NULL,
    disposition character varying,
    settlement jsonb,
    created_at_unix bigint NOT NULL,
    updated_at_unix bigint NOT NULL,
    resolved_at_unix bigint
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

CREATE INDEX idx_scope_request_events_request_time ON scope_request_events USING btree (request_id, created_at_unix);


--
-- Name: idx_scope_requests_author; Type: INDEX; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

CREATE INDEX idx_scope_requests_author ON scope_requests USING btree (author_user_id);


--
-- Name: idx_scope_requests_repo_state; Type: INDEX; Schema: scope_test_2249234_1783653779131957768; Owner: -
--

CREATE INDEX idx_scope_requests_repo_state ON scope_requests USING btree (repo_id, state);


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
        stake_credits >= 0 AND created_at_unix >= 0 AND updated_at_unix >= 0 AND
        (resolved_at_unix IS NULL OR resolved_at_unix >= 0)
    ),
    ADD CONSTRAINT scope_request_identity_values CHECK (
        name ~ '^[a-z0-9][a-z0-9-]{0,47}$' AND
        name NOT IN ('main', 'head', 'scope') AND
        audience IN ('Public', 'Private')
    );
ALTER TABLE scope_request_events
    ADD CONSTRAINT scope_request_event_time CHECK (created_at_unix >= 0);
ALTER TABLE scope_user_credit_accounts
    ADD CONSTRAINT scope_user_credit_balance CHECK (balance_credits >= 0);
ALTER TABLE scope_credit_ledger_entries
    ADD CONSTRAINT scope_credit_ledger_entry_time CHECK (created_at_unix >= 0);
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
