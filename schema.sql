CREATE TABLE company_batches(
    business_name text unique not null,
    description text,
    start_date integer not null default(strftime('%s', 'now')),
    end_date integer
);
CREATE TABLE ssn_host_data(
    ssn integer primary key,
    primary_acct integer,
    info_codes text,
    created_account integer,
    host_err text
);
CREATE TABLE ssn_batch_relat(
    batch_id not null references company_batches(rowid),
    ssn not null references ssn_host_data(ssn)
);
create table envelopes(
    gid text not null primary key,
    ssn integer not null references acct_map(ssn),
    status text,
    api_err text,
    fname text,
    mname text,
    lname text,
    dob date,
    addr1 text,
    addr2 text,
    city text,
    sate text,
    zip text,
    email text,
    phone text,
    spouse_fname text,
    spouse_mname text,
    spouse_lname text,
    spouse_email text,
    date_created date not null default(strftime('%s', 'now')),
    pdf blob
);
CREATE TABLE beneficiaries(
    gid text not null primary key references envelopes(gid),
    --primary/contingent
    type text,
    name text,
    address text,
    city_state_zip text,
    dob date,
    relationship text,
    ssn integer,
    percent integer
);
CREATE TABLE authorized_users(
    gid text not null primary key references envelopes(gid),
    name text,
    dob date
);
