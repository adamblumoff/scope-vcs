import { CopyableCodeBlock } from '@/components/copyable-code-block'
import {
  ToggleGroup,
  ToggleGroupItem,
} from '@/components/ui/toggle-group'
import { useState } from 'react'
import {
  defaultGitCommandShell,
  gitCommandShellOptions,
  isGitCommandShell,
  type GitCommandShell,
} from './git-command-shell'

export function GitCommandBlock({
  copyLabel,
  value,
}: {
  copyLabel?: string
  value: (shell: GitCommandShell) => string
}) {
  const [shell, setShell] = useState<GitCommandShell>(defaultGitCommandShell)

  return (
    <div className="min-w-0 space-y-2">
      <ToggleGroup
        aria-label="Command shell"
        onValueChange={(nextShell) => {
          if (isGitCommandShell(nextShell)) {
            setShell(nextShell)
          }
        }}
        size="sm"
        type="single"
        value={shell}
      >
        {gitCommandShellOptions.map((option) => (
          <ToggleGroupItem
            aria-label={`${option.label} command`}
            key={option.value}
            value={option.value}
          >
            {option.label}
          </ToggleGroupItem>
        ))}
      </ToggleGroup>
      <CopyableCodeBlock copyLabel={copyLabel} value={value(shell)} />
    </div>
  )
}
