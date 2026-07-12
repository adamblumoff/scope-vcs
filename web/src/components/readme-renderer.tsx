import { cn } from '@/lib/utils'
import type { ComponentProps } from 'react'
import rehypeSlug from 'rehype-slug'
import Markdown from 'react-markdown'
import remarkGfm from 'remark-gfm'
import { resolveReadmeUrl } from './readme-url'

export function ReadmeRenderer({
  className,
  repository,
  source,
}: {
  className?: string
  repository: { owner: string; readmePath: string; repo: string }
  source: string
}) {
  return (
    <article
      className={cn(
        'mx-auto w-full max-w-[760px] px-6 py-8 text-[15px] leading-7 text-foreground sm:px-10 sm:py-10',
        className,
      )}
    >
      <Markdown
        components={markdownComponents}
        rehypePlugins={[[rehypeSlug, { prefix: 'readme-' }]]}
        remarkPlugins={[remarkGfm]}
        skipHtml
        urlTransform={(url) => resolveReadmeUrl(url, repository)}
      >
        {source}
      </Markdown>
    </article>
  )
}

const markdownComponents = {
  a: ({ children, className, href, ...props }: ComponentProps<'a'>) =>
    href ? (
      <a
        className={cn(
          'font-medium text-foreground underline decoration-[var(--platinum)]/70 underline-offset-4 hover:decoration-[var(--platinum-bright)]',
          className,
        )}
        href={href}
        rel="noreferrer"
        {...props}
      >
        {children}
      </a>
    ) : (
      <span className={className}>{children}</span>
    ),
  blockquote: ({ className, ...props }: ComponentProps<'blockquote'>) => (
    <blockquote
      className={cn(
        'my-6 border-l-2 border-[var(--platinum)] pl-4 text-muted-foreground',
        className,
      )}
      {...props}
    />
  ),
  code: ({ className, ...props }: ComponentProps<'code'>) => (
    <code
      className={cn(
        'rounded bg-muted px-1.5 py-0.5 font-mono text-[0.88em] text-foreground',
        className,
      )}
      {...props}
    />
  ),
  h1: ({ children, className, ...props }: ComponentProps<'h1'>) => (
    <h1
      className={cn(
        'mb-5 border-b border-border pb-5 text-3xl font-semibold leading-tight tracking-[-0.03em] sm:text-[36px]',
        className,
      )}
      {...props}
    >
      {children}
    </h1>
  ),
  h2: ({ children, className, ...props }: ComponentProps<'h2'>) => (
    <h2
      className={cn(
        'mb-3 mt-9 border-b border-border pb-3 text-2xl font-semibold tracking-[-0.025em]',
        className,
      )}
      {...props}
    >
      {children}
    </h2>
  ),
  h3: ({ children, className, ...props }: ComponentProps<'h3'>) => (
    <h3
      className={cn('mb-2 mt-7 text-xl font-semibold tracking-[-0.02em]', className)}
      {...props}
    >
      {children}
    </h3>
  ),
  hr: ({ className, ...props }: ComponentProps<'hr'>) => (
    <hr className={cn('my-8 border-border', className)} {...props} />
  ),
  img: ({ alt }: ComponentProps<'img'>) => (
    <span className="my-4 block border-l-2 border-border pl-3 text-sm italic text-muted-foreground">
      Image omitted{alt ? `: ${alt}` : ''}
    </span>
  ),
  li: ({ className, ...props }: ComponentProps<'li'>) => (
    <li className={cn('my-1.5 pl-1', className)} {...props} />
  ),
  ol: ({ className, ...props }: ComponentProps<'ol'>) => (
    <ol className={cn('my-5 list-decimal pl-6', className)} {...props} />
  ),
  p: ({ className, ...props }: ComponentProps<'p'>) => (
    <p className={cn('my-4 text-pretty', className)} {...props} />
  ),
  pre: ({ className, ...props }: ComponentProps<'pre'>) => (
    <pre
      className={cn(
        'my-6 overflow-x-auto rounded-lg border border-border border-l-2 border-l-[var(--platinum)] bg-[#090b0e] p-4 font-mono text-xs leading-6 text-[#eceae5] shadow-[var(--shadow-card)] [&_code]:bg-transparent [&_code]:p-0 [&_code]:text-inherit',
        className,
      )}
      {...props}
    />
  ),
  table: ({ className, ...props }: ComponentProps<'table'>) => (
    <div className="my-6 overflow-x-auto rounded-lg border border-border">
      <table className={cn('w-full border-collapse text-sm', className)} {...props} />
    </div>
  ),
  td: ({ className, ...props }: ComponentProps<'td'>) => (
    <td className={cn('border-b border-border px-3 py-2.5', className)} {...props} />
  ),
  th: ({ className, ...props }: ComponentProps<'th'>) => (
    <th
      className={cn(
        'border-b border-border bg-muted px-3 py-2.5 text-left text-xs font-semibold',
        className,
      )}
      {...props}
    />
  ),
  ul: ({ className, ...props }: ComponentProps<'ul'>) => (
    <ul className={cn('my-5 list-disc pl-6', className)} {...props} />
  ),
}
