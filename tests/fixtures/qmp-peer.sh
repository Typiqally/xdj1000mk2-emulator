#!/bin/sh
printf '%s\n' '{"QMP":{}}'
while IFS= read -r line; do
    if test -n "$MOCK_QMP_LOG"; then printf '%s\n' "$line" >> "$MOCK_QMP_LOG"; fi
    id=$(printf '%s' "$line" | sed -E 's/.*"id":([0-9]+).*/\1/')
    action=$(printf '%s' "$line" | sed -E 's/.*"execute":"([^"]+)".*/\1/')
    case "$action" in
        qom-list) value='[{"name":"xdj-live-state"},{"name":"xdj-panel-frame"}]' ;;
        qom-get) value='{"running":true,"stop":false,"consumed_frames":100,"dropped_frames":0,"queue_recoveries":0,"serializer_underruns":0}' ;;
        *) value='{}' ;;
    esac
    case "$line" in
        *'"property":"xdj-live-state"'*) value='{"audio":{"running":true,"stop":false,"consumed_frames":100,"dropped_frames":0,"queue_recoveries":0,"serializer_underruns":0},"performance":{},"age_us":0}' ;;
        *'"property":"xdj-panel-frame"'*) value='"{\"frame\":\"\"}"' ;;
    esac
    printf '{"id":%s,"return":%s}\n'  "$id" "$value"
    if test "$action" = quit; then exit 0; fi
done
