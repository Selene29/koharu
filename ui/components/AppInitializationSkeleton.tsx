'use client'

import { useMemo } from 'react'
import { useTranslation } from 'react-i18next'
import { useListDownloads } from '@/lib/api/downloads/downloads'
import type { BootstrapStatus } from '@/lib/api/schemas'
import { Progress } from '@/components/ui/progress'
import { AlertCircle } from 'lucide-react'
import type { DownloadState } from '@/lib/api/schemas'

const summarizeDownloads = (downloads?: DownloadState[] | null) => {
  if (!downloads?.length) return null
  let total = 0
  let downloaded = 0
  let active: string | null = null
  for (const d of downloads) {
    total += d.total ?? 0
    downloaded += d.downloaded
    if (d.status === 'started' || d.status === 'downloading')
      active = d.filename
  }
  return {
    filename: active,
    percent:
      total > 0
        ? Math.min(100, Math.round((downloaded / total) * 100))
        : undefined,
  }
}

type AppInitializationSkeletonProps = {
  bootstrap?: BootstrapStatus
}

export function AppInitializationSkeleton({
  bootstrap,
}: AppInitializationSkeletonProps) {
  const { t } = useTranslation()
  const { data: downloads } = useListDownloads({
    query: { refetchInterval: 1500 },
  })

  const progress = useMemo(() => summarizeDownloads(downloads), [downloads])
  const retrying = bootstrap?.phase === 'retrying'

  return (
    <div className='bg-background flex min-h-0 flex-1 items-center justify-center px-6 py-10'>
      <div className='flex w-full max-w-[560px] flex-col items-center gap-6'>
        <img
          src='/icon-large.png'
          alt='Koharu'
          className='h-20 w-20 opacity-80'
          draggable={false}
        />

        <div className='flex flex-col items-center gap-1'>
          <h1 className='text-foreground text-lg font-semibold tracking-widest uppercase'>
            Koharu
          </h1>
          <p className='text-muted-foreground text-xs'>
            {retrying ? t('bootstrap.retrying') : t('common.initializing')}
          </p>
        </div>

        <div className='w-56'>
          <p className='text-muted-foreground mb-1.5 h-4 truncate text-center text-[11px]'>
            {progress?.filename ?? '\u00A0'}
          </p>
          <Progress
            value={progress?.percent ?? 0}
            className={`h-1.5 ${progress ? 'visible' : 'invisible'}`}
          />
        </div>

        {retrying && (
          <div className='border-border bg-card w-full rounded-xl border p-5'>
            <div className='flex items-start gap-3'>
              <div className='mt-0.5 flex size-9 shrink-0 items-center justify-center rounded-full bg-red-500/10 text-red-500'>
                <AlertCircle className='size-4.5' />
              </div>
              <div className='min-w-0 flex-1 space-y-3'>
                <div className='space-y-1'>
                  <h2 className='text-sm font-semibold'>
                    {t('bootstrap.failedPrepareRuntime')}
                  </h2>
                  <p className='text-muted-foreground text-sm leading-relaxed'>
                    {t('bootstrap.retryingDescription')}
                  </p>
                </div>
                {bootstrap?.error && (
                  <div className='space-y-2'>
                    <div className='text-muted-foreground text-xs font-medium tracking-wide uppercase'>
                      {t('bootstrap.errorDetails')}
                    </div>
                    <pre className='bg-muted text-foreground max-h-56 overflow-auto rounded-md px-3 py-2 text-xs break-words whitespace-pre-wrap'>
                      {bootstrap.error}
                    </pre>
                  </div>
                )}
              </div>
            </div>
          </div>
        )}
      </div>
    </div>
  )
}
