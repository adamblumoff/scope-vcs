import { cn } from '@/lib/utils'
import type { ComponentProps } from 'react'
import Markdown from 'react-markdown'

export const compactMarkdownComponents = {
  a: ({ children, className, href, ...props }: ComponentProps<'a'>) =>
    href ? (
      <a
        className={cn(
          'font-medium text-foreground underline decoration-border-strong underline-offset-4 hover:decoration-foreground',
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
        'my-3 border-l-2 border-border-strong pl-3 text-muted-foreground',
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
    <h1 className={cn('mb-3 mt-5 text-xl font-semibold', className)} {...props}>
      {children}
    </h1>
  ),
  h2: ({ children, className, ...props }: ComponentProps<'h2'>) => (
    <h2 className={cn('mb-2 mt-5 text-lg font-semibold', className)} {...props}>
      {children}
    </h2>
  ),
  h3: ({ children, className, ...props }: ComponentProps<'h3'>) => (
    <h3 className={cn('mb-2 mt-4 text-base font-semibold', className)} {...props}>
      {children}
    </h3>
  ),
  hr: ({ className, ...props }: ComponentProps<'hr'>) => (
    <hr className={cn('my-4 border-border', className)} {...props} />
  ),
  img: ({ alt }: ComponentProps<'img'>) => (
    <span className="my-3 block border-l-2 border-border pl-3 text-sm italic text-muted-foreground">
      Image omitted{alt ? `: ${alt}` : ''}
    </span>
  ),
  li: ({ className, ...props }: ComponentProps<'li'>) => (
    <li className={cn('my-1 pl-1', className)} {...props} />
  ),
  ol: ({ className, ...props }: ComponentProps<'ol'>) => (
    <ol className={cn('my-3 list-decimal pl-6', className)} {...props} />
  ),
  p: ({ className, ...props }: ComponentProps<'p'>) => (
    <p className={cn('my-2 text-pretty', className)} {...props} />
  ),
  pre: ({ className, ...props }: ComponentProps<'pre'>) => (
    <pre
      className={cn(
        'my-3 overflow-x-auto rounded-md border border-border bg-[#090b0e] p-3 font-mono text-xs leading-5 text-[#eceae5] [&_code]:bg-transparent [&_code]:p-0 [&_code]:text-inherit',
        className,
      )}
      {...props}
    />
  ),
  table: ({ className, ...props }: ComponentProps<'table'>) => (
    <div className="my-3 overflow-x-auto border-y border-border">
      <table className={cn('w-full border-collapse text-sm', className)} {...props} />
    </div>
  ),
  td: ({ className, ...props }: ComponentProps<'td'>) => (
    <td className={cn('border-b border-border px-3 py-2', className)} {...props} />
  ),
  th: ({ className, ...props }: ComponentProps<'th'>) => (
    <th
      className={cn(
        'border-b border-border bg-muted px-3 py-2 text-left text-xs font-semibold',
        className,
      )}
      {...props}
    />
  ),
  ul: ({ className, ...props }: ComponentProps<'ul'>) => (
    <ul className={cn('my-3 list-disc pl-6', className)} {...props} />
  ),
} satisfies NonNullable<ComponentProps<typeof Markdown>['components']>
