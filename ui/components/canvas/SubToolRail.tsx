'use client'

import { AnimatePresence, motion } from 'motion/react'
import { X, Brush, Eraser, Bandage, ChevronDown, ChevronRight } from 'lucide-react'
import * as React from 'react'
import { useTranslation } from 'react-i18next'

import { ColorPicker } from '@/components/ui/color-picker'
import { Slider } from '@/components/ui/slider'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { useEditorUiStore } from '@/lib/stores/editorUiStore'
import { usePreferencesStore } from '@/lib/stores/preferencesStore'
import { cn } from '@/lib/utils'

export function SubToolRail() {
  const mode = useEditorUiStore((state) => state.mode)
  const setMode = useEditorUiStore((state) => state.setMode)
  const isBrushTool = mode === 'brush' || mode === 'eraser' || mode === 'repairBrush'
  
  const brushConfig = usePreferencesStore((state) => state.brushConfig)
  const setBrushConfig = usePreferencesStore((state) => state.setBrushConfig)
  const { t } = useTranslation()

  // Local state for live updates
  const [localSize, setLocalSize] = React.useState(brushConfig.size)

  // Sync when store changes
  React.useEffect(() => {
    setLocalSize(brushConfig.size)
  }, [brushConfig.size])

  if (!isBrushTool) return null

  return (
    <AnimatePresence>
      <motion.div
        initial={{ x: -20, opacity: 0 }}
        animate={{ x: 0, opacity: 1 }}
        exit={{ x: -20, opacity: 0 }}
        transition={{ duration: 0.2, ease: 'easeOut' }}
        className='absolute left-11 top-14 z-50 ml-1 flex flex-col w-[260px] rounded-xl border border-border bg-card shadow-2xl overflow-hidden'
        data-testid='sub-tool-rail'
      >
        <div className='p-4 space-y-4'>
          {/* Brush Size */}
          <div className='space-y-2'>
            <label className='text-[11px] font-medium text-muted-foreground'>{t('toolbar.brushSize')}</label>
            <div className='flex items-center gap-2'>
              <Slider
                min={8}
                max={128}
                step={4}
                value={[localSize]}
                onValueChange={(vals) => setLocalSize(vals[0] ?? localSize)}
                onValueCommit={(vals) => setBrushConfig({ size: vals[0] ?? localSize })}
                className='flex-1'
              />
              <div className='flex items-center gap-1.5 shrink-0'>
                <Input 
                  value={localSize} 
                  readOnly 
                  className='h-8 w-11 text-[11px] text-center bg-muted/20 border-border/50 px-1'
                />
                <span className='text-[10px] font-medium text-muted-foreground w-4'>px</span>
              </div>
            </div>
          </div>

          {/* Color Picker Section */}
          <AnimatePresence initial={false}>
            {mode === 'brush' && (
              <motion.div
                initial={{ height: 0, opacity: 0 }}
                animate={{ height: 'auto', opacity: 1 }}
                exit={{ height: 0, opacity: 0 }}
                transition={{ duration: 0.2, ease: 'easeInOut' }}
                className='overflow-hidden border-t border-border/30 pt-2'
              >
                 <div className='flex items-center justify-between'>
                    <label className='text-[11px] font-medium text-muted-foreground'>{t('toolbar.brushColor')}</label>
                    <div className='flex items-center gap-2'>
                       <span className='text-[10px] font-mono text-muted-foreground uppercase'>{brushConfig.color}</span>
                       <ColorPicker
                          value={brushConfig.color}
                          onChange={(color) => setBrushConfig({ color })}
                          className='size-5 rounded-md'
                        />
                    </div>
                 </div>
              </motion.div>
            )}
          </AnimatePresence>
        </div>
      </motion.div>
    </AnimatePresence>
  )
}
