# PipelineConfig

## Properties

Name | Type | Description | Notes
------------ | ------------- | ------------- | -------------
**bubble_segmenter** | Option<**String**> |  | [optional][default to speech-bubble-segmentation]
**detector** | Option<**String**> |  | [optional][default to pp-doclayout-v3]
**font_detector** | Option<**String**> |  | [optional][default to yuzumarker-font-detection]
**inpainter** | Option<**String**> |  | [optional][default to lama-manga]
**ocr** | Option<**String**> |  | [optional][default to paddle-ocr-vl-1.5]
**parallelism** | Option<[**models::PipelineParallelismConfig**](PipelineParallelismConfig.md)> |  | [optional]
**renderer** | Option<**String**> |  | [optional][default to koharu-renderer]
**segmenter** | Option<**String**> |  | [optional][default to comic-text-detector-seg]
**translator** | Option<**String**> |  | [optional][default to llm]

[[Back to Model list]](../README.md#documentation-for-models) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to README]](../README.md)


