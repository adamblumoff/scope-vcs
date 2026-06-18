create table users (
  id text primary key,
  email text not null unique,
  email_verified boolean not null default false,
  access text not null default 'public' check (access in ('public', 'member')),
  created_at timestamptz not null default now()
);

create table repos (
  id text primary key,
  owner_handle text not null,
  name text not null,
  owner_user_id text not null references users(id),
  publication_state text not null default 'unpublished' check (publication_state in ('unpublished', 'published')),
  default_visibility text not null default 'public' check (default_visibility in ('public', 'private')),
  created_at timestamptz not null default now(),
  unique (owner_handle, name)
);

create table repo_memberships (
  repo_id text not null references repos(id) on delete cascade,
  user_id text not null references users(id) on delete cascade,
  role text not null check (role in ('reader', 'writer', 'maintainer', 'owner')),
  created_at timestamptz not null default now(),
  primary key (repo_id, user_id)
);

create table repo_invitations (
  id uuid primary key,
  repo_id text not null references repos(id) on delete cascade,
  invited_email text not null,
  role text not null check (role in ('reader', 'writer', 'maintainer', 'owner')),
  invited_by_user_id text not null references users(id),
  state text not null default 'pending' check (state in ('pending', 'accepted', 'revoked')),
  created_at timestamptz not null default now()
);

create table path_nodes (
  id uuid primary key,
  repo_id text not null references repos(id) on delete cascade,
  parent_id uuid references path_nodes(id),
  path text not null,
  kind text not null check (kind in ('file', 'dir')),
  unique (repo_id, path)
);

create table visibility_rules (
  id uuid primary key,
  repo_id text not null references repos(id) on delete cascade,
  path_node_id uuid not null references path_nodes(id),
  visibility text not null check (visibility in ('public', 'private')),
  allowed_user_ids jsonb not null default '[]'::jsonb,
  created_at timestamptz not null default now()
);

create table blobs (
  id uuid primary key,
  repo_id text not null references repos(id) on delete cascade,
  digest text not null,
  encrypted_content_pointer text not null,
  size_bytes bigint not null,
  unique (repo_id, digest)
);

create table logical_commits (
  id text primary key,
  repo_id text not null references repos(id) on delete cascade,
  parent_ids jsonb not null default '[]'::jsonb,
  author_user_id text not null references users(id),
  author_visibility text not null,
  message_ciphertext text,
  created_at timestamptz not null default now()
);

create table commit_changes (
  commit_id text not null references logical_commits(id) on delete cascade,
  path_node_id uuid not null references path_nodes(id),
  old_blob_id uuid references blobs(id),
  new_blob_id uuid references blobs(id),
  change_type text not null,
  primary key (commit_id, path_node_id)
);

create table push_manifests (
  id uuid primary key,
  repo_id text not null references repos(id) on delete cascade,
  user_id text not null references users(id),
  device_id text not null,
  commit_graph_hash text not null,
  mixed_policy text not null,
  signature text not null,
  created_at timestamptz not null default now()
);

create table projection_commits (
  repo_id text not null references repos(id) on delete cascade,
  principal_scope text not null,
  logical_commit_id text not null references logical_commits(id),
  git_sha text not null,
  primary key (repo_id, principal_scope, logical_commit_id)
);

create table audit_events (
  id uuid primary key,
  repo_id text not null references repos(id) on delete cascade,
  actor_user_id text references users(id),
  action text not null,
  object_path text,
  revision_id text,
  result text not null,
  reason text,
  created_at timestamptz not null default now()
);
