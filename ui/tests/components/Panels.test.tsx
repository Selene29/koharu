import { screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it } from 'vitest'

import { Panels } from '@/components/Panels'
import { useEditorUiStore } from '@/lib/stores/editorUiStore'

import { renderWithQuery } from '../helpers'

describe('Panels / BrushPanel Integration', () => {
  beforeEach(() => {
    // Reset mode to a default non-brush state
    useEditorUiStore.getState().setMode('select')
  })

  it('hides BrushPanel when a non-drawing tool is selected', async () => {
    renderWithQuery(<Panels />)
    expect(screen.queryByTestId('panels-brush')).not.toBeInTheDocument()
  })

  it('shows BrushPanel when drawing tools are selected', async () => {
    const drawingModes = ['brush', 'eraser', 'repairBrush'] as const

    for (const mode of drawingModes) {
      useEditorUiStore.getState().setMode(mode)
      const { unmount } = renderWithQuery(<Panels />)
      expect(screen.getByTestId('panels-brush')).toBeInTheDocument()
      unmount()
    }
  })

  it('shows the color picker section ONLY when the brush tool is selected', async () => {
    // 1. Brush mode: Color picker should be present
    useEditorUiStore.getState().setMode('brush')
    const { rerender } = renderWithQuery(<Panels />)
    expect(screen.getByTestId('brush-color-section')).toBeInTheDocument()

    // 2. Eraser mode: Panel exists, but color picker is hidden
    useEditorUiStore.getState().setMode('eraser')
    rerender(<Panels />)
    expect(screen.getByTestId('panels-brush')).toBeInTheDocument()
    await waitFor(() => expect(screen.queryByTestId('brush-color-section')).not.toBeInTheDocument())

    // 3. RepairBrush mode: Panel exists, but color picker is hidden
    useEditorUiStore.getState().setMode('repairBrush')
    rerender(<Panels />)
    expect(screen.getByTestId('panels-brush')).toBeInTheDocument()
    await waitFor(() => expect(screen.queryByTestId('brush-color-section')).not.toBeInTheDocument())
  })
})
