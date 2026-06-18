create table principals (
  id text primary key,
  kind text not null,
  created_at timestamptz not null default now()
);

create table repos (
  id text primary key,
  owner_id text not null references principals(id),
  default_visibility text not null check (default_visibility in ('public', 'private')),
  created_at timestamptz not null default now()
);

create table path_nodes (
  id uuid primary key,
  repo_id text not null references repos(id),
  parent_id uuid references path_nodes(id),
  path text not null,
  kind text not null check (kind in ('file', 'dir')),
  unique (repo_id, path)
);

create table visibility_rules (
  id uuid primary key,
  repo_id text not null references repos(id),
  path_node_id uuid not null references path_nodes(id),
  visibility text not null check (visibility in ('public', 'private')),
  allowed_principals jsonb not null default '[]'::jsonb,
  created_at timestamptz not null default now()
);

create table blobs (
  id uuid primary key,
  repo_id text not null references repos(id),
  digest text not null,
  encrypted_content_pointer text not null,
  size_bytes bigint not null,
  unique (repo_id, digest)
);

create table logical_commits (
  id text primary key,
  repo_id text not null references repos(id),
  parent_ids jsonb not null default '[]'::jsonb,
  author_id text not null references principals(id),
  author_visibility text not null,
  message_ciphertext text,
  created_at timestamptz not null default now()
);

create table commit_changes (
  commit_id text not null references logical_commits(id),
  path_node_id uuid not null references path_nodes(id),
  old_blob_id uuid references blobs(id),
  new_blob_id uuid references blobs(id),
  change_type text not null,
  primary key (commit_id, path_node_id)
);

create table push_manifests (
  id uuid primary key,
  repo_id text not null references repos(id),
  user_id text not null references principals(id),
  device_id text not null,
  commit_graph_hash text not null,
  mixed_policy text not null,
  signature text not null,
  created_at timestamptz not null default now()
);

create table projection_commits (
  repo_id text not null references repos(id),
  principal_scope text not null,
  logical_commit_id text not null references logical_commits(id),
  git_sha text not null,
  primary key (repo_id, principal_scope, logical_commit_id)
);

create table audit_events (
  id uuid primary key,
  repo_id text not null references repos(id),
  actor_id text not null,
  action text not null,
  object_path text,
  revision_id text,
  result text not null,
  reason text,
  created_at timestamptz not null default now()
);

