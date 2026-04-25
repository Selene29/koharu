import { create } from 'zustand'

export type LogLevel = 'info' | 'warn' | 'error'

export type LogEntry = {
  id: number
  timestamp: number
  level: LogLevel
  message: string
  detail?: string
}

const MAX_ENTRIES = 5000

let nextId = 1

type ActivityLogState = {
  entries: LogEntry[]
  push: (level: LogLevel, message: string, detail?: string) => void
  clear: () => void
}

export const useActivityLogStore = create<ActivityLogState>((set) => ({
  entries: [],
  push: (level, message, detail) =>
    set((state) => {
      const entry: LogEntry = { id: nextId++, timestamp: Date.now(), level, message, detail }
      const next = [...state.entries, entry]
      if (next.length > MAX_ENTRIES) next.splice(0, next.length - MAX_ENTRIES)
      return { entries: next }
    }),
  clear: () => set({ entries: [] }),
}))
