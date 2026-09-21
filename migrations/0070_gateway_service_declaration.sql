-- Persist the immutable, typed declaration for the long-lived HTTP service
-- contract. Runtime state and readiness remain separate concerns.
ALTER TABLE gateway_revisions
    DROP CONSTRAINT IF EXISTS gateway_revisions_handler_contract_check;

ALTER TABLE gateway_revisions
    ADD COLUMN service_loopback_port integer,
    ADD COLUMN service_readiness_path text,
    ADD COLUMN service_health_path text;

ALTER TABLE gateway_revisions
    ADD CONSTRAINT gateway_revisions_handler_contract_check
        CHECK (handler_contract IN ('http.v1', 'http.service.v1')),
    ADD CONSTRAINT gateway_revisions_service_shape_check
        CHECK (
            (handler_contract = 'http.v1'
                AND service_loopback_port IS NULL
                AND service_readiness_path IS NULL
                AND service_health_path IS NULL)
            OR
            (handler_contract = 'http.service.v1'
                AND service_loopback_port IS NOT NULL
                AND service_loopback_port BETWEEN 1024 AND 65535
                AND service_readiness_path IS NOT NULL
                AND service_health_path IS NOT NULL)
        ),
    ADD CONSTRAINT gateway_revisions_service_paths_check
        CHECK (
            handler_contract <> 'http.service.v1'
            OR (
                octet_length(service_readiness_path) BETWEEN 1 AND 512
                AND service_readiness_path LIKE '/%'
                AND service_readiness_path NOT LIKE '%?%'
                AND service_readiness_path NOT LIKE '%#%'
                AND service_readiness_path NOT LIKE '%\%%'
                AND service_readiness_path NOT LIKE '%\\%'
                AND service_readiness_path NOT LIKE '%*%'
                AND position('//' IN service_readiness_path) = 0
                AND service_readiness_path !~ '[[:space:][:cntrl:]]'
                AND service_readiness_path !~ '(^|/)\.\.?(/|$)'
                AND (octet_length(service_readiness_path) = 1 OR right(service_readiness_path, 1) <> '/')
                AND octet_length(service_health_path) BETWEEN 1 AND 512
                AND service_health_path LIKE '/%'
                AND service_health_path NOT LIKE '%?%'
                AND service_health_path NOT LIKE '%#%'
                AND service_health_path NOT LIKE '%\%%'
                AND service_health_path NOT LIKE '%\\%'
                AND service_health_path NOT LIKE '%*%'
                AND position('//' IN service_health_path) = 0
                AND service_health_path !~ '[[:space:][:cntrl:]]'
                AND service_health_path !~ '(^|/)\.\.?(/|$)'
                AND (octet_length(service_health_path) = 1 OR right(service_health_path, 1) <> '/')
            )
        );
