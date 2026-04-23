'use client'

import { Trash2Icon } from 'lucide-react'
import { useEffect, useRef } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { useActivityLogStore, type LogEntry } from '@/lib/stores/activityLogStore'

const LEVEL_STYLE: Record<string, string> = {
  info: 'bg-emerald-500',
  warn: 'bg-amber-500',
  error: 'bg-red-500',
}

function formatTime(ts: number): string {
  const d = new Date(ts)
  return d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' })
}

function EntryRow({ entry }: { entry: LogEntry }) {
  return (
    <div className='flex items-start gap-2 px-2 py-0.5 text-[11px] leading-relaxed hover:bg-muted/50'>
      <span className={`mt-1.5 size-1.5 shrink-0 rounded-full ${LEVEL_STYLE[entry.level] ?? LEVEL_STYLE.info}`} />
      <span className='shrink-0 font-mono text-muted-foreground'>{formatTime(entry.timestamp)}</span>
      <span className={entry.level === 'error' ? 'text-red-500' : entry.level === 'warn' ? 'text-amber-600' : 'text-foreground'}>
        {entry.message}
      </span>
      {entry.detail && (
        <span className='truncate text-muted-foreground'>— {entry.detail}</span>
      )}
    </div>
  )
}

export function ActivityLogPanel() {
  const { t } = useTranslation()
  const entries = useActivityLogStore((s) => s.entries)
  const clear = useActivityLogStore((s) => s.clear)
  const bottomRef = useRef<HTMLDivElement>(null)

  // Auto-scroll to bottom when new entries arrive
  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: 'smooth' })
  }, [entries.length])

  return (
    <div className='flex h-full min-h-0 flex-col bg-card'>
      <div className='flex shrink-0 items-center justify-between border-b border-border px-2 py-1'>
        <span className='text-[10px] font-semibold tracking-wide text-muted-foreground uppercase'>
          {t('activityLog.title', { defaultValue: 'Activity Log' })}
        </span>
        <Button
          variant='ghost'
          size='icon'
          className='size-5'
          onClick={clear}
          title={t('activityLog.clear', { defaultValue: 'Clear' })}
        >
          <Trash2Icon className='size-3' />
        </Button>
      </div>
      <div className='min-h-0 flex-1 overflow-y-auto font-mono'>
        {entries.length === 0 ? (
          <div className='px-2 py-4 text-center text-xs text-muted-foreground'>
            {t('activityLog.empty', { defaultValue: 'No activity yet' })}
          </div>
        ) : (
          entries.map((entry) => <EntryRow key={entry.id} entry={entry} />)
        )}
        <div ref={bottomRef} />
      </div>
    </div>
  )
}
