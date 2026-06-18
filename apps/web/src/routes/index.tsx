import { createFileRoute } from '@tanstack/react-router'
import {
  AlertCircle,
  CheckCircle2,
  GitBranch,
  KeyRound,
  Lock,
  RefreshCw,
  Server,
  ShieldCheck,
  Upload,
} from 'lucide-react'
import { useEffect, useState } from 'react'

export const Route = createFileRoute('/')({
  component: ScopeDashboard,
})

type PrincipalId = 'public' | 'team-core' | 'owner'

type ProjectedChange = {
  path: string
  new_content: string | null
}

type ProjectedCommit = {
  projected_id: string
  logical_commit_id: string
  parent_projected_id: string | null
  author: string | null
  message: string
  synthetic: boolean
  changes: ProjectedChange[]
}

type Projection = {
  repo_id: string
  principal_id: string
  commits: ProjectedCommit[]
}

type GitProjection = {
  principal_id: string
  blobs: Array<{
    path: string
    oid: string
    content: string
  }>
  head_oid: string | null
}

type HealthResponse = {
  status: string
  service: string
}

type ManifestResponse = {
  signed_manifest: {
    manifest: {
      id: string
      repo_id: string
      principal_id: string
      device_id: string
      commit_graph_hash: string
      changed_paths: string[]
      mixed_policy: string
    }
    signature_hex: string
  }
}

type LoadState<T> = {
  data: T | null
  error: string | null
  loading: boolean
}

const repoId = 'scope-demo'
const deployedApiBase = 'https://scope-api-production-0251.up.railway.app'

const principals: Array<{
  id: PrincipalId
  label: string
  detail: string
}> = [
  {
    id: 'public',
    label: 'Public',
    detail: 'No private names, bytes, authors, counts, or cadence.',
  },
  {
    id: 'team-core',
    label: 'Team Core',
    detail: 'Authorized for the internal boundary.',
  },
  {
    id: 'owner',
    label: 'Owner',
    detail: 'Full canonical view and write authority.',
  },
]

const ownerPolicyRows = [
  {
    path: '/',
    visibility: 'public',
    access: 'default',
    note: 'Public objects are readable, not automatically writable.',
  },
  {
    path: '/internal',
    visibility: 'private',
    access: 'owner, team-core',
    note: 'Top-down boundary; no public islands in v1.',
  },
]

function ScopeDashboard() {
  const [principal, setPrincipal] = useState<PrincipalId>('public')
  const [projection, setProjection] = useState<LoadState<Projection>>({
    data: null,
    error: null,
    loading: true,
  })
  const [gitProjection, setGitProjection] = useState<LoadState<GitProjection>>({
    data: null,
    error: null,
    loading: true,
  })
  const [health, setHealth] = useState<LoadState<HealthResponse>>({
    data: null,
    error: null,
    loading: true,
  })
  const [manifest, setManifest] = useState<LoadState<ManifestResponse>>({
    data: null,
    error: null,
    loading: false,
  })
  const [gitBoundary, setGitBoundary] = useState<{
    state: 'checking' | 'explicit' | 'unexpected' | 'error'
    detail: string
  }>({
    state: 'checking',
    detail: 'checking smart HTTP boundary',
  })

  const [baseUrl, setBaseUrl] = useState(deployedApiBase)

  useEffect(() => {
    setBaseUrl(getStaticApiBase())
  }, [])

  useEffect(() => {
    const controller = new AbortController()
    setProjection({ data: null, error: null, loading: true })
    setGitProjection({ data: null, error: null, loading: true })
    setManifest({ data: null, error: null, loading: false })

    Promise.all([
      loadJson<Projection>(
        `${baseUrl}/v1/repos/${repoId}/projections/${principal}`,
        controller.signal,
      ),
      loadJson<GitProjection>(
        `${baseUrl}/v1/repos/${repoId}/git-projections/${principal}`,
        controller.signal,
      ),
    ])
      .then(([projectionData, gitData]) => {
        setProjection({ data: projectionData, error: null, loading: false })
        setGitProjection({ data: gitData, error: null, loading: false })
      })
      .catch((error: unknown) => {
        if (controller.signal.aborted) {
          return
        }
        const message = error instanceof Error ? error.message : 'request failed'
        setProjection({ data: null, error: message, loading: false })
        setGitProjection({ data: null, error: message, loading: false })
      })

    return () => controller.abort()
  }, [baseUrl, principal])

  useEffect(() => {
    const controller = new AbortController()

    loadJson<HealthResponse>(`${baseUrl}/healthz`, controller.signal)
      .then((data) => setHealth({ data, error: null, loading: false }))
      .catch((error: unknown) => {
        if (controller.signal.aborted) {
          return
        }
        setHealth({
          data: null,
          error: error instanceof Error ? error.message : 'health check failed',
          loading: false,
        })
      })

    fetch(
      `${baseUrl}/git/acme/${repoId}/info/refs?service=git-upload-pack`,
      { signal: controller.signal },
    )
      .then(async (response) => {
        const body = await response.json().catch(() => null)
        if (response.status === 501) {
          setGitBoundary({
            state: 'explicit',
            detail:
              body?.next ??
              'Git clone is blocked until real packfile serving exists.',
          })
          return
        }
        setGitBoundary({
          state: 'unexpected',
          detail: `unexpected status ${response.status}`,
        })
      })
      .catch((error: unknown) => {
        if (controller.signal.aborted) {
          return
        }
        setGitBoundary({
          state: 'error',
          detail:
            error instanceof Error
              ? error.message
              : 'git boundary check failed',
        })
      })

    return () => controller.abort()
  }, [baseUrl])

  const visiblePaths = projection.data
    ? visibleProjectionPaths(projection.data)
    : []
  const selectedPrincipal = principals.find((item) => item.id === principal)
  const omittedCount = principal === 'public' ? 1 : 0

  async function createManifest() {
    setManifest({ data: null, error: null, loading: true })
    const changed_paths =
      principal === 'team-core' ? ['/internal/model.rs'] : ['/README.md']

    try {
      const response = await fetch(
        `${baseUrl}/v1/repos/${repoId}/push-manifests`,
        {
          method: 'POST',
          headers: { 'content-type': 'application/json' },
          body: JSON.stringify({
            principal_id: principal,
            device_id: 'web-demo',
            commit_graph_hash: `${principal}-demo-graph`,
            changed_paths,
            mixed_policy: 'SyntheticPublicCommit',
          }),
        },
      )
      const payload = await response.json().catch(() => null)

      if (!response.ok) {
        throw new Error(payload?.error ?? `request failed: ${response.status}`)
      }

      setManifest({
        data: payload as ManifestResponse,
        error: null,
        loading: false,
      })
    } catch (error) {
      setManifest({
        data: null,
        error: error instanceof Error ? error.message : 'manifest failed',
        loading: false,
      })
    }
  }

  return (
    <main className="shell">
      <section className="console" aria-labelledby="scope-title">
        <div className="topbar">
          <div>
            <p className="eyebrow">scope-vcs</p>
            <h1 id="scope-title">Scope</h1>
          </div>
          <ServiceStrip health={health} gitBoundary={gitBoundary} />
        </div>

        <div className="workspace-grid">
          <aside className="principal-rail" aria-label="Projection principals">
            <div className="repo-block">
              <span>repo</span>
              <strong>{repoId}</strong>
              <small suppressHydrationWarning>
                {baseUrl.replace(/^https?:\/\//, '')}
              </small>
            </div>

            <div className="principal-list" role="tablist">
              {principals.map((item) => (
                <button
                  aria-selected={principal === item.id}
                  className="principal-button"
                  key={item.id}
                  onClick={() => setPrincipal(item.id)}
                  role="tab"
                  type="button"
                >
                  <span>{item.label}</span>
                  <small>{item.detail}</small>
                </button>
              ))}
            </div>

            <div className="boundary-note">
              <Lock size={16} />
              <span>
                Projection views never disclose private paths outside the
                selected principal&apos;s scope.
              </span>
            </div>
          </aside>

          <section className="projection-stage" aria-live="polite">
            <div className="stage-heading">
              <div>
                <p className="eyebrow">selected projection</p>
                <h2>{selectedPrincipal?.label}</h2>
              </div>
              <StatusBadge
                state={projection.loading ? 'loading' : projection.error ? 'bad' : 'good'}
                text={
                  projection.loading
                    ? 'loading'
                    : projection.error
                      ? 'unavailable'
                      : `${projection.data?.commits.length ?? 0} commits`
                }
              />
            </div>

            <div className="projection-summary">
              <Metric
                label="visible paths"
                value={projection.loading ? '...' : visiblePaths.length}
              />
              <Metric
                label="virtual blobs"
                value={
                  gitProjection.loading
                    ? '...'
                    : gitProjection.data?.blobs.length ?? 0
                }
              />
              <Metric
                label="synthetic commits"
                value={
                  projection.loading
                    ? '...'
                    : projection.data?.commits.filter((commit) => commit.synthetic)
                        .length ?? 0
                }
              />
              <Metric label="omitted for public" value={omittedCount} />
            </div>

            {projection.error ? (
              <Notice tone="bad" text={projection.error} />
            ) : (
              <CommitTimeline loading={projection.loading} projection={projection.data} />
            )}
          </section>
        </div>
      </section>

      <section className="details-grid" aria-label="Scope control plane">
        <section className="detail-band">
          <div className="section-heading">
            <div>
              <p className="eyebrow">projection output</p>
              <h2>Visible Object Set</h2>
            </div>
            <GitBranch size={20} />
          </div>
          <ObjectSet
            gitProjection={gitProjection}
            principal={principal}
            visiblePaths={visiblePaths}
          />
        </section>

        <section className="detail-band">
          <div className="section-heading">
            <div>
              <p className="eyebrow">admin-only</p>
              <h2>Policy Boundary</h2>
            </div>
            <ShieldCheck size={20} />
          </div>
          <div className="policy-table">
            {ownerPolicyRows.map((row) => (
              <div className="policy-row" key={row.path}>
                <code>{row.path}</code>
                <strong data-visibility={row.visibility}>{row.visibility}</strong>
                <span>{row.access}</span>
                <small>{row.note}</small>
              </div>
            ))}
          </div>
        </section>

        <section className="detail-band">
          <div className="section-heading">
            <div>
              <p className="eyebrow">write path</p>
              <h2>Push Manifest</h2>
            </div>
            <KeyRound size={20} />
          </div>
          <div className="manifest-panel">
            <button
              className="manifest-button"
              disabled={manifest.loading}
              onClick={createManifest}
              type="button"
            >
              {manifest.loading ? <RefreshCw size={18} /> : <Upload size={18} />}
              Create demo manifest
            </button>
            <ManifestResult manifest={manifest} principal={principal} />
          </div>
        </section>

        <section className="detail-band">
          <div className="section-heading">
            <div>
              <p className="eyebrow">compatibility</p>
              <h2>Git Boundary</h2>
            </div>
            <Server size={20} />
          </div>
          <div className="git-boundary">
            <StatusBadge
              state={gitBoundary.state === 'explicit' ? 'good' : 'bad'}
              text={
                gitBoundary.state === 'explicit'
                  ? 'honest 501'
                  : gitBoundary.state
              }
            />
            <p>{gitBoundary.detail}</p>
          </div>
        </section>
      </section>
    </main>
  )
}

function ServiceStrip({
  health,
  gitBoundary,
}: {
  health: LoadState<HealthResponse>
  gitBoundary: { state: 'checking' | 'explicit' | 'unexpected' | 'error' }
}) {
  const apiGood = Boolean(health.data && !health.error)
  const services = [
    {
      name: 'scope-web',
      detail: 'page loaded',
      state: 'online',
      good: true,
    },
    {
      name: 'scope-api',
      detail: health.loading
        ? 'checking'
        : health.data?.service ?? health.error ?? 'offline',
      state: apiGood ? 'online' : health.loading ? 'checking' : 'offline',
      good: apiGood,
    },
    {
      name: 'git facade',
      detail: 'clone blocked until packs exist',
      state: gitBoundary.state === 'explicit' ? 'guarded' : 'check',
      good: gitBoundary.state === 'explicit',
    },
    {
      name: 'postgres / bucket',
      detail: 'Railway infrastructure',
      state: 'private',
      good: true,
    },
  ]

  return (
    <div className="service-strip" aria-label="Service status">
      {services.map((service) => (
        <div className="service-pill" data-good={service.good} key={service.name}>
          {service.good ? <CheckCircle2 size={16} /> : <AlertCircle size={16} />}
          <span>{service.name}</span>
          <small>{service.state}</small>
        </div>
      ))}
    </div>
  )
}

function CommitTimeline({
  loading,
  projection,
}: {
  loading: boolean
  projection: Projection | null
}) {
  if (loading) {
    return (
      <div className="timeline">
        {['one', 'two', 'three'].map((item) => (
          <div className="commit-row skeleton" key={item} />
        ))}
      </div>
    )
  }

  if (!projection || projection.commits.length === 0) {
    return <Notice tone="neutral" text="No commits are visible to this principal." />
  }

  return (
    <div className="timeline">
      {projection.commits.map((commit) => (
        <div className="commit-row" data-synthetic={commit.synthetic} key={commit.projected_id}>
          <span>{commit.logical_commit_id}</span>
          <strong>{commit.message}</strong>
          <em>{commit.synthetic ? 'synthetic' : 'canonical'}</em>
          <code>{commit.projected_id}</code>
          <small>
            {commit.author ?? 'author hidden'} · {commit.changes.length} visible change
            {commit.changes.length === 1 ? '' : 's'}
          </small>
        </div>
      ))}
    </div>
  )
}

function ObjectSet({
  gitProjection,
  principal,
  visiblePaths,
}: {
  gitProjection: LoadState<GitProjection>
  principal: PrincipalId
  visiblePaths: string[]
}) {
  if (gitProjection.error) {
    return <Notice tone="bad" text={gitProjection.error} />
  }

  const blobs = gitProjection.data?.blobs ?? []

  return (
    <div className="object-set">
      <div className="path-column">
        <span>visible paths</span>
        {visiblePaths.length === 0 && !gitProjection.loading ? (
          <p>No paths visible.</p>
        ) : (
          visiblePaths.map((path) => <code key={path}>{path}</code>)
        )}
      </div>
      <div className="blob-column">
        <span>virtual Git blobs</span>
        {gitProjection.loading ? (
          <p>Loading object set...</p>
        ) : (
          blobs.map((blob) => (
            <div className="blob-row" key={`${blob.path}-${blob.oid}`}>
              <code>{blob.oid.slice(0, 12)}</code>
              <span>{blob.path}</span>
            </div>
          ))
        )}
      </div>
      {principal === 'public' ? (
        <Notice
          tone="good"
          text="The public object set contains no private path names or private bytes."
        />
      ) : null}
    </div>
  )
}

function ManifestResult({
  manifest,
  principal,
}: {
  manifest: LoadState<ManifestResponse>
  principal: PrincipalId
}) {
  if (manifest.error) {
    return (
      <Notice
        tone="bad"
        text={`${principal} request rejected: ${manifest.error}`}
      />
    )
  }

  if (!manifest.data) {
    return (
      <p className="manifest-empty">
        Public principals should be rejected for writes. Authorized principals
        receive a signed manifest before push.
      </p>
    )
  }

  const signed = manifest.data.signed_manifest

  return (
    <div className="manifest-result">
      <span>signed manifest</span>
      <code>{signed.manifest.id}</code>
      <small>{signed.signature_hex.slice(0, 24)}...</small>
    </div>
  )
}

function Metric({ label, value }: { label: string; value: number | string }) {
  return (
    <div className="metric">
      <strong>{value}</strong>
      <span>{label}</span>
    </div>
  )
}

function StatusBadge({
  state,
  text,
}: {
  state: 'good' | 'bad' | 'loading'
  text: string
}) {
  return (
    <span className="status-badge" data-state={state}>
      {state === 'good' ? <CheckCircle2 size={15} /> : <AlertCircle size={15} />}
      {text}
    </span>
  )
}

function Notice({ tone, text }: { tone: 'good' | 'bad' | 'neutral'; text: string }) {
  return (
    <div className="notice" data-tone={tone}>
      {tone === 'bad' ? <AlertCircle size={17} /> : <CheckCircle2 size={17} />}
      <span>{text}</span>
    </div>
  )
}

function visibleProjectionPaths(projection: Projection) {
  const paths = projection.commits.flatMap((commit) =>
    commit.changes.map((change) => change.path),
  )
  return [...new Set(paths)].sort((left, right) => left.localeCompare(right))
}

async function loadJson<T>(url: string, signal: AbortSignal): Promise<T> {
  const response = await fetch(url, { signal })
  const payload = await response.json().catch(() => null)

  if (!response.ok) {
    throw new Error(payload?.error ?? `request failed: ${response.status}`)
  }

  return payload as T
}

function getStaticApiBase() {
  const envBase = import.meta.env.VITE_SCOPE_API_URL as string | undefined
  if (envBase) {
    return stripTrailingSlash(envBase)
  }

  return deployedApiBase
}

function stripTrailingSlash(value: string) {
  return value.replace(/\/+$/, '')
}
