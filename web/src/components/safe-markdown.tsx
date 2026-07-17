import type { ComponentProps } from 'react'
import Markdown, { type UrlTransform } from 'react-markdown'
import remarkGfm from 'remark-gfm'
import { cn } from '@/lib/utils'

export function SafeMarkdown({
  children,
  className,
  components,
  rehypePlugins,
  urlTransform,
}: {
  children: string
  className?: string
  components?: ComponentProps<typeof Markdown>['components']
  rehypePlugins?: ComponentProps<typeof Markdown>['rehypePlugins']
  urlTransform?: UrlTransform
}) {
  return (
    <div className={cn(className)}>
      <Markdown
        components={components}
        rehypePlugins={rehypePlugins}
        remarkPlugins={[remarkGfm]}
        skipHtml
        urlTransform={urlTransform}
      >
        {children}
      </Markdown>
    </div>
  )
}
