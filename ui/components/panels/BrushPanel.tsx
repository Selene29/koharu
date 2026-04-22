'use client'

import { AnimatePresence, motion } from 'motion/react'
import { useTranslation } from 'react-i18next'

import { ColorPicker } from '@/components/ui/color-picker'
import { Slider } from '@/components/ui/slider'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { useEditorUiStore } from '@/lib/stores/editorUiStore'
import { usePreferencesStore } from '@/lib/stores/preferencesStore'

export function BrushPanel() {
  const showColorPicker = useEditorUiStore((state) => state.mode === 'brush')
  const brushSize = usePreferencesStore((state) => state.brushConfig.size)
  const brushColor = usePreferencesStore((state) => state.brushConfig.color)
  const decreaseShortcut = usePreferencesStore((state) => state.shortcuts.decreaseBrushSize)
  const increaseShortcut = usePreferencesStore((state) => state.shortcuts.increaseBrushSize)
  const setBrushConfig = usePreferencesStore((state) => state.setBrushConfig)
  const { t } = useTranslation()

  return (
    <div className='flex flex-col border-b border-border' data-testid='panels-brush'>
      <div className='space-y-2 p-3'>
        <p className='text-[10px] font-semibold text-muted-foreground uppercase tracking-wider'>
          {t('toolbar.brushSize')}
        </p>
        <div className='flex items-center gap-3'>
          <Slider
            data-testid='brush-size-slider'
            className='flex-1 [&_[data-slot=slider-range]]:bg-primary [&_[data-slot=slider-thumb]]:size-3 [&_[data-slot=slider-thumb]]:border-primary [&_[data-slot=slider-thumb]]:bg-primary [&_[data-slot=slider-track]]:bg-primary/20'
            min={8}
            max={128}
            step={4}
            value={[brushSize]}
            onValueChange={(vals) => setBrushConfig({ size: vals[0] ?? brushSize })}
          />
          <Tooltip>
            <TooltipTrigger asChild>
              <span className='min-w-10 cursor-help text-right text-xs text-muted-foreground tabular-nums'>
                {brushSize}px
              </span>
            </TooltipTrigger>
            <TooltipContent side='left'>
              {t('toolbar.brushSize')} ({decreaseShortcut}{' '}
              {increaseShortcut})
            </TooltipContent>
          </Tooltip>
        </div>
      </div>

      <AnimatePresence initial={false}>
        {showColorPicker && (
          <motion.div
            initial={{ height: 0, opacity: 0 }}
            animate={{ height: 'auto', opacity: 1 }}
            exit={{ height: 0, opacity: 0 }}
            transition={{ duration: 0.2, ease: 'easeInOut' }}
            className='overflow-hidden border-t border-border/50 bg-muted/20'
          >
            <div className='space-y-2 p-3'>
              <p className='text-[10px] font-semibold text-muted-foreground uppercase tracking-wider'>
                {t('toolbar.brushColor')}
              </p>
              <div className='flex items-center gap-3'>
                <ColorPicker
                  value={brushColor}
                  onChange={(color) => setBrushConfig({ color })}
                  className='h-8 w-8 rounded-md border border-border shadow-sm transition-shadow hover:shadow-md'
                  triggerTestId='brush-color-trigger'
                  pickerTestId='brush-color-picker'
                  swatchTestId='brush-color-swatch'
                  inputTestId='brush-color-input'
                  pickButtonTestId='brush-color-pick'
                />
                <span className='text-xs font-medium text-muted-foreground tabular-nums uppercase'>
                  {brushColor}
                </span>
              </div>
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  )
}
