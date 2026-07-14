import type { CliInstallCommands, CliPlatform } from '@/api/types'
import { CopyableCodeBlock } from '@/components/copyable-code-block'
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group'
import { CheckCircle2 } from 'lucide-react'
import { useRef, useState, type ReactElement } from 'react'

const platformOptions = [
  { copyName: 'macOS and Linux', label: 'macOS / Linux', value: 'posix' },
  { copyName: 'Windows', label: 'Windows', value: 'windows' },
] as const satisfies ReadonlyArray<{
  copyName: string
  label: string
  value: CliPlatform
}>

const nextSteps = [
  {
    command: 'scope login',
    copyLabel: 'Copy login command',
    description: 'Sign in from your terminal. This opens your browser.',
  },
  {
    command: 'scope init',
    copyLabel: 'Copy init command',
    description: 'Run from an existing Git repository with at least one commit.',
  },
  {
    command: 'scope push',
    copyLabel: 'Copy push command',
    description: 'Review and publish the repository’s first version.',
  },
] as const

export function MarketingCliOnboarding({
  commands,
  initialPlatform,
}: {
  commands: CliInstallCommands
  initialPlatform: CliPlatform
}): ReactElement {
  const [platform, setPlatform] = useState<CliPlatform>(initialPlatform)
  const [showNextSteps, setShowNextSteps] = useState(false)
  const nextStepsHeadingRef = useRef<HTMLHeadingElement>(null)

  const installCommand = commands[platform]
  const platformOption = platformOptions.find(
    (option) => option.value === platform,
  ) ?? platformOptions[0]

  function revealNextSteps(moveFocus = false) {
    setShowNextSteps(true)
    if (moveFocus) {
      window.requestAnimationFrame(() => nextStepsHeadingRef.current?.focus())
    }
  }

  return (
    <section aria-labelledby="install-scope" className="mt-9 max-w-[570px]">
      <div className="mb-3 flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between">
        <div>
          <h2 className="text-sm font-semibold" id="install-scope">
            Install Scope
          </h2>
          <p className="mt-1 text-xs text-muted-foreground">
            Install now. Sign in when you connect a repository.
          </p>
        </div>
        <ToggleGroup
          aria-label="Operating system"
          className="grid w-full grid-cols-2 sm:w-auto"
          onValueChange={(value) => {
            if (value) {
              setPlatform(value as CliPlatform)
            }
          }}
          type="single"
          value={platform}
        >
          {platformOptions.map((option) => (
            <ToggleGroupItem
              className="px-3 text-xs"
              key={option.value}
              value={option.value}
            >
              {option.label}
            </ToggleGroupItem>
          ))}
        </ToggleGroup>
      </div>

      <CopyableCodeBlock
        className="shadow-[0_18px_55px_-34px_rgba(0,0,0,0.9)]"
        copyLabel={`Copy ${platformOption.copyName} install command`}
        key={platform}
        onCopy={revealNextSteps}
        value={installCommand}
      />

      {!showNextSteps && (
        <button
          className="mt-3 text-xs text-muted-foreground underline decoration-border-strong underline-offset-4 transition-colors hover:text-foreground"
          onClick={() => revealNextSteps(true)}
          type="button"
        >
          Already installed? Show next steps
        </button>
      )}

      {showNextSteps && (
        <div
          aria-live="polite"
          className="marketing-cli-next-steps mt-5 border-t border-border pt-5"
        >
          <div className="mb-4 flex items-center gap-2 font-mono text-[11px] font-semibold text-[var(--success-strong)]">
            <CheckCircle2 className="size-3.5" />
            Ready for the next step
          </div>
          <h3
            className="text-sm font-semibold outline-none"
            ref={nextStepsHeadingRef}
            tabIndex={-1}
          >
            Connect a repository
          </h3>
          <div className="mt-4 space-y-4">
            {nextSteps.map((step, index) => (
              <div
                className="grid grid-cols-[22px_minmax(0,1fr)] gap-2.5"
                key={step.command}
              >
                <span className="mt-0.5 grid size-[22px] place-items-center rounded-full border border-border font-mono text-[9px] text-muted-foreground">
                  {index + 1}
                </span>
                <div className="min-w-0">
                  <p className="mb-2 text-xs leading-5 text-muted-foreground">
                    {step.description}
                  </p>
                  <CopyableCodeBlock
                    className="shadow-none"
                    copyLabel={step.copyLabel}
                    value={step.command}
                  />
                </div>
              </div>
            ))}
          </div>
        </div>
      )}
    </section>
  )
}
