-- Processing locks table for distributed deduplication
-- This table ensures that only one instance processes a repository at a time

-- Define the processing_lock table
DEFINE TABLE processing_lock SCHEMAFULL;

-- Define fields
DEFINE FIELD repo_id ON TABLE processing_lock TYPE record<repo> ASSERT $value IS NOT NULL;
DEFINE FIELD instance_id ON TABLE processing_lock TYPE string ASSERT $value IS NOT NULL;
DEFINE FIELD locked_at ON TABLE processing_lock TYPE datetime ASSERT $value IS NOT NULL;
DEFINE FIELD expires_at ON TABLE processing_lock TYPE datetime ASSERT $value IS NOT NULL;
DEFINE FIELD processing_status ON TABLE processing_lock TYPE string 
    ASSERT $value IN ["processing", "completed", "failed"];

-- Create unique index on repo_id to ensure only one lock per repo
DEFINE INDEX idx_processing_lock_repo ON TABLE processing_lock COLUMNS repo_id UNIQUE;

-- Create index on expires_at for efficient cleanup of expired locks
DEFINE INDEX idx_processing_lock_expires ON TABLE processing_lock COLUMNS expires_at;

-- Create index on instance_id for querying locks by instance
DEFINE INDEX idx_processing_lock_instance ON TABLE processing_lock COLUMNS instance_id;

-- Function to acquire a lock (returns true if successful, false if already locked)
DEFINE FUNCTION fn::acquire_processing_lock($repo_id: record<repo>, $instance_id: string, $lock_duration_seconds: int) {
    -- Default lock duration is 5 minutes
    LET $duration = IF $lock_duration_seconds THEN $lock_duration_seconds ELSE 300;
    LET $now = time::now();
    LET $expires = time::now() + duration($duration + "s");
    
    -- Try to create a lock
    -- If a lock already exists and hasn't expired, this will fail due to unique constraint
    IF (SELECT * FROM processing_lock WHERE repo_id = $repo_id AND expires_at > $now) {
        RETURN false;
    } ELSE {
        -- Delete any expired lock for this repo
        DELETE processing_lock WHERE repo_id = $repo_id AND expires_at <= $now;
        
        -- Create new lock
        CREATE processing_lock SET
            repo_id = $repo_id,
            instance_id = $instance_id,
            locked_at = $now,
            expires_at = $expires,
            processing_status = "processing";
        
        RETURN true;
    }
};

-- Function to release a lock (only the instance that owns it can release it)
DEFINE FUNCTION fn::release_processing_lock($repo_id: record<repo>, $instance_id: string, $status: string) {
    UPDATE processing_lock 
    SET processing_status = $status, expires_at = time::now()
    WHERE repo_id = $repo_id AND instance_id = $instance_id;
};

-- Function to clean up expired locks
DEFINE FUNCTION fn::cleanup_expired_locks() {
    DELETE processing_lock WHERE expires_at <= time::now();
};

-- Function to extend a lock (for long-running operations)
DEFINE FUNCTION fn::extend_processing_lock($repo_id: record<repo>, $instance_id: string, $additional_seconds: int) {
    LET $additional = IF $additional_seconds THEN $additional_seconds ELSE 300;
    
    UPDATE processing_lock 
    SET expires_at = expires_at + duration($additional + "s")
    WHERE repo_id = $repo_id 
        AND instance_id = $instance_id 
        AND expires_at > time::now()
    RETURN AFTER;
};