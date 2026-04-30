# JobLogEvent

## Properties

Name | Type | Description | Notes
------------ | ------------- | ------------- | -------------
**detail** | Option<**String**> |  | [optional]
**job_id** | **String** |  | 
**level** | [**models::JobLogLevel**](JobLogLevel.md) |  | 
**message** | **String** |  | 
**page_index** | Option<**u32**> | 0-based page index this log refers to. `None` for global messages. | [optional]
**step_id** | Option<**String**> | Engine id (e.g. `\"paddle-ocr\"`) when applicable. | [optional]
**total_pages** | **u32** |  | 

[[Back to Model list]](../README.md#documentation-for-models) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to README]](../README.md)


