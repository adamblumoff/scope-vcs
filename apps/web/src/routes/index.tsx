import { createFileRoute } from '@tanstack/react-router'
import {
  CheckCircle2,
  GitBranch,
  Globe2,
  KeyRound,
  Lock,
  ShieldCheck,
  Upload,
} from 'lucide-react'

export const Route = createFileRoute('/')({
  component: ScopeDashboard,
})

const paths = [
  {
    path: '/README.md',
    visibility: 'public',
    principal: 'public',
    status: 'projected',
  },
  {
    path: '/docs/api.md',
    visibility: 'public',
    principal: 'public',
    status: 'synthetic',
  },
  {
    path: '/internal/model.rs',
    visibility: 'private',
    principal: 'team-core',
    status: 'withheld',
  },
]

const commits = [
  {
    id: 'rv_001',
    scope: 'public',
    message: 'Initialize public surface',
    result: 'P1',
  },
  {
    id: 'rv_002',
    scope: 'mixed',
    message: 'Synthetic public projection',
    result: 'P2',
  },
  {
    id: 'rv_003',
    scope: 'private',
    message: 'Omitted from public projection',
    result: 'hidden',
  },
]

const services = [
  ['scope-api', 'Axum API', 'ready'],
  ['scope-worker', 'Projection jobs', 'ready'],
  ['scope-web', 'TanStack Start', 'ready'],
  ['Postgres', 'Canonical metadata', 'pending env'],
  ['Bucket', 'Encrypted blobs', 'pending env'],
]

function ScopeDashboard() {
  return (
    <main className="shell">
      <section className="hero" aria-labelledby="scope-title">
        <div className="hero-copy">
          <p className="eyebrow">scope-vcs</p>
          <h1 id="scope-title">Scope</h1>
          <p className="lede">
            Permissioned source graphs with Git-compatible projections.
          </p>
          <div className="actions">
            <button title="Create push manifest">
              <Upload size={18} />
              Manifest
            </button>
            <button title="Review policy state" className="secondary">
              <ShieldCheck size={18} />
              Policy
            </button>
          </div>
        </div>
        <div className="projection-plane" aria-label="Projection status">
          <div className="plane-header">
            <span>public projection</span>
            <Globe2 size={18} />
          </div>
          <div className="graph-line">
            <span>P1</span>
            <span>P2</span>
            <span className="muted">hidden</span>
          </div>
          <div className="plane-footer">
            <Lock size={16} />
            <span>No private path names, counts, or cadence exposed.</span>
          </div>
        </div>
      </section>

      <section className="workspace" aria-label="Scope control plane">
        <aside className="rail">
          <div>
            <p className="rail-label">repo</p>
            <strong>scope-demo</strong>
          </div>
          <nav>
            <a href="#paths">Paths</a>
            <a href="#history">History</a>
            <a href="#railway">Railway</a>
          </nav>
        </aside>

        <div className="surface">
          <section id="paths" className="band">
            <div className="section-heading">
              <div>
                <p className="eyebrow">visibility</p>
                <h2>Path State</h2>
              </div>
              <span className="badge">top-down</span>
            </div>
            <div className="path-list">
              {paths.map((item) => (
                <div className="path-row" key={item.path}>
                  <span className={`status-dot ${item.visibility}`} />
                  <code>{item.path}</code>
                  <span>{item.principal}</span>
                  <strong>{item.status}</strong>
                </div>
              ))}
            </div>
          </section>

          <section id="history" className="band">
            <div className="section-heading">
              <div>
                <p className="eyebrow">projection</p>
                <h2>Commit Mapping</h2>
              </div>
              <GitBranch size={20} />
            </div>
            <div className="timeline">
              {commits.map((commit) => (
                <div className="commit-row" key={commit.id}>
                  <span>{commit.id}</span>
                  <strong>{commit.message}</strong>
                  <em>{commit.scope}</em>
                  <code>{commit.result}</code>
                </div>
              ))}
            </div>
          </section>

          <section id="railway" className="band">
            <div className="section-heading">
              <div>
                <p className="eyebrow">deploy</p>
                <h2>Railway Shape</h2>
              </div>
              <KeyRound size={20} />
            </div>
            <div className="service-grid">
              {services.map(([name, role, state]) => (
                <div className="service-row" key={name}>
                  <CheckCircle2 size={18} />
                  <strong>{name}</strong>
                  <span>{role}</span>
                  <em>{state}</em>
                </div>
              ))}
            </div>
          </section>
        </div>
      </section>
    </main>
  )
}

