//! Log aggregation framework

use std::cmp;

use k8s_openapi::api::core::v1::ResourceRequirements;

use crate::{
    builder::ContainerBuilder, commons::product_image_selection::ResolvedProductImage,
    k8s_openapi::api::core::v1::Container, kube::Resource, role_utils::RoleGroupRef,
};

use super::spec::{
    AutomaticContainerLogConfig, ContainerLogConfig, ContainerLogConfigChoice, LogLevel,
};

/// Config directory used in the Vector log agent container
const STACKABLE_CONFIG_DIR: &str = "/stackable/config";
/// Directory in the Vector container which contains a subdirectory for every container which
/// themselves contain the corresponding log files
const STACKABLE_LOG_DIR: &str = "/stackable/log";
/// Subdirectory of the log directory containing files to control the Vector instance
const VECTOR_LOG_DIR: &str = "_vector";
/// File to signal that Vector should be gracefully shut down
const SHUTDOWN_FILE: &str = "shutdown";

/// File name of the Vector config file
pub const VECTOR_CONFIG_FILE: &str = "vector.toml";

/// Create a Bash command which filters stdout and stderr according to the given log configuration
/// and additionally stores the output in log files
///
/// # Example
///
/// ```
/// use stackable_operator::{
///     builder::ContainerBuilder,
///     config::fragment,
///     product_logging,
///     product_logging::spec::{
///         ContainerLogConfig, ContainerLogConfigChoice, Logging,
///     },
/// };
/// # use stackable_operator::product_logging::spec::default_logging;
/// # use strum::{Display, EnumIter};
/// #
/// # #[derive(Clone, Display, Eq, EnumIter, Ord, PartialEq, PartialOrd)]
/// # pub enum Container {
/// #     Init,
/// # }
/// #
/// # let logging = fragment::validate::<Logging<Container>>(default_logging()).unwrap();
///
/// const STACKABLE_LOG_DIR: &str = "/stackable/log";
///
/// let mut args = Vec::new();
///
/// if let Some(ContainerLogConfig {
///     choice: Some(ContainerLogConfigChoice::Automatic(log_config)),
/// }) = logging.containers.get(&Container::Init)
/// {
///     args.push(product_logging::framework::capture_shell_output(
///         STACKABLE_LOG_DIR,
///         "init",
///         &log_config,
///     ));
/// }
/// args.push("echo Test".into());
///
/// let init_container = ContainerBuilder::new("init")
///     .unwrap()
///     .command(vec!["bash".to_string(), "-c".to_string()])
///     .args(vec![args.join(" && ")])
///     .build();
/// ```
pub fn capture_shell_output(
    log_dir: &str,
    container: &str,
    log_config: &AutomaticContainerLogConfig,
) -> String {
    let root_log_level = log_config.root_log_level();
    let console_log_level = cmp::max(
        root_log_level,
        log_config
            .console
            .as_ref()
            .and_then(|console| console.level)
            .unwrap_or_default(),
    );
    let file_log_level = cmp::max(
        root_log_level,
        log_config
            .file
            .as_ref()
            .and_then(|file| file.level)
            .unwrap_or_default(),
    );

    let log_file_dir = format!("{log_dir}/{container}");

    let stdout_redirect = match (
        console_log_level <= LogLevel::INFO,
        file_log_level <= LogLevel::INFO,
    ) {
        (true, true) => format!(" > >(tee {log_file_dir}/container.stdout.log)"),
        (true, false) => "".into(),
        (false, true) => format!(" > {log_file_dir}/container.stdout.log"),
        (false, false) => " > /dev/null".into(),
    };

    let stderr_redirect = match (
        console_log_level <= LogLevel::ERROR,
        file_log_level <= LogLevel::ERROR,
    ) {
        (true, true) => format!(" 2> >(tee {log_file_dir}/container.stderr.log >&2)"),
        (true, false) => "".into(),
        (false, true) => format!(" 2> {log_file_dir}/container.stderr.log"),
        (false, false) => " 2> /dev/null".into(),
    };

    let mut args = Vec::new();
    if file_log_level <= LogLevel::ERROR {
        args.push(format!("mkdir --parents {log_file_dir}"));
    }
    if stdout_redirect.is_empty() && stderr_redirect.is_empty() {
        args.push(":".into());
    } else {
        args.push(format!("exec{stdout_redirect}{stderr_redirect}"));
    }

    args.join(" && ")
}

/// Create the content of a log4j properties file according to the given log configuration
///
/// # Arguments
///
/// * `log_dir` - Directory where the log files are stored
/// * `log_file` - Name of the active log file; When the file is rolled over then a number is
///       appended.
/// * `max_size_in_mib` - Maximum size of all log files in MiB; This value can be slightly
///       exceeded. The value is set to 2 if the given value is lower (1 MiB for the active log
///       file and 1 MiB for the archived one).
/// * `console_conversion_pattern` - Logback conversion pattern for the console appender
/// * `config` - The logging configuration for the container
///
/// # Example
///
/// ```
/// use stackable_operator::{
///     builder::{
///         ConfigMapBuilder,
///         meta::ObjectMetaBuilder,
///     },
///     config::fragment,
///     product_logging,
///     product_logging::spec::{
///         ContainerLogConfig, ContainerLogConfigChoice, Logging,
///     },
/// };
/// # use stackable_operator::product_logging::spec::default_logging;
/// # use strum::{Display, EnumIter};
/// #
/// # #[derive(Clone, Display, Eq, EnumIter, Ord, PartialEq, PartialOrd)]
/// # pub enum Container {
/// #     MyProduct,
/// # }
/// #
/// # let logging = fragment::validate::<Logging<Container>>(default_logging()).unwrap();
///
/// const STACKABLE_LOG_DIR: &str = "/stackable/log";
/// const LOG4J_CONFIG_FILE: &str = "log4j.properties";
/// const MY_PRODUCT_LOG_FILE: &str = "my-product.log4j.xml";
/// const MAX_LOG_FILE_SIZE_IN_MIB: u32 = 10;
/// const CONSOLE_CONVERSION_PATTERN: &str = "%d{ISO8601} %-5p %m%n";
///
/// let mut cm_builder = ConfigMapBuilder::new();
/// cm_builder.metadata(ObjectMetaBuilder::default().build());
///
/// if let Some(ContainerLogConfig {
///     choice: Some(ContainerLogConfigChoice::Automatic(log_config)),
/// }) = logging.containers.get(&Container::MyProduct)
/// {
///     cm_builder.add_data(
///         LOG4J_CONFIG_FILE,
///         product_logging::framework::create_log4j_config(
///             &format!("{STACKABLE_LOG_DIR}/my-product"),
///             MY_PRODUCT_LOG_FILE,
///             MAX_LOG_FILE_SIZE_IN_MIB,
///             CONSOLE_CONVERSION_PATTERN,
///             log_config,
///         ),
///     );
/// }
///
/// cm_builder.build().unwrap();
/// ```
pub fn create_log4j_config(
    log_dir: &str,
    log_file: &str,
    max_size_in_mib: u32,
    console_conversion_pattern: &str,
    config: &AutomaticContainerLogConfig,
) -> String {
    let number_of_archived_log_files = 1;

    let loggers = config
        .loggers
        .iter()
        .filter(|(name, _)| name.as_str() != AutomaticContainerLogConfig::ROOT_LOGGER)
        .map(|(name, logger_config)| {
            format!(
                "log4j.logger.{name}={level}\n",
                name = name.escape_default(),
                level = logger_config.level.to_log4j_literal(),
            )
        })
        .collect::<String>();

    format!(
        r#"log4j.rootLogger={root_log_level}, CONSOLE, FILE

log4j.appender.CONSOLE=org.apache.log4j.ConsoleAppender
log4j.appender.CONSOLE.Threshold={console_log_level}
log4j.appender.CONSOLE.layout=org.apache.log4j.PatternLayout
log4j.appender.CONSOLE.layout.ConversionPattern={console_conversion_pattern}

log4j.appender.FILE=org.apache.log4j.RollingFileAppender
log4j.appender.FILE.Threshold={file_log_level}
log4j.appender.FILE.File={log_dir}/{log_file}
log4j.appender.FILE.MaxFileSize={max_log_file_size_in_mib}MB
log4j.appender.FILE.MaxBackupIndex={number_of_archived_log_files}
log4j.appender.FILE.layout=org.apache.log4j.xml.XMLLayout

{loggers}"#,
        max_log_file_size_in_mib =
            cmp::max(1, max_size_in_mib / (1 + number_of_archived_log_files)),
        root_log_level = config.root_log_level().to_log4j_literal(),
        console_log_level = config
            .console
            .as_ref()
            .and_then(|console| console.level)
            .unwrap_or_default()
            .to_log4j_literal(),
        file_log_level = config
            .file
            .as_ref()
            .and_then(|file| file.level)
            .unwrap_or_default()
            .to_log4j_literal(),
    )
}

/// Create the content of a log4j2 properties file according to the given log configuration
///
/// # Arguments
///
/// * `log_dir` - Directory where the log files are stored
/// * `log_file` - Name of the active log file; When the file is rolled over then a number is
///       appended.
/// * `max_size_in_mib` - Maximum size of all log files in MiB; This value can be slightly
///       exceeded. The value is set to 2 if the given value is lower (1 MiB for the active log
///       file and 1 MiB for the archived one).
/// * `console_conversion_pattern` - Log4j2 conversion pattern for the console appender
/// * `config` - The logging configuration for the container
///
/// # Example
///
/// ```
/// use stackable_operator::{
///     builder::{
///         ConfigMapBuilder,
///         meta::ObjectMetaBuilder,
///     },
///     config::fragment,
///     product_logging,
///     product_logging::spec::{
///         ContainerLogConfig, ContainerLogConfigChoice, Logging,
///     },
/// };
/// # use stackable_operator::product_logging::spec::default_logging;
/// # use strum::{Display, EnumIter};
/// #
/// # #[derive(Clone, Display, Eq, EnumIter, Ord, PartialEq, PartialOrd)]
/// # pub enum Container {
/// #     MyProduct,
/// # }
/// #
/// # let logging = fragment::validate::<Logging<Container>>(default_logging()).unwrap();
///
/// const STACKABLE_LOG_DIR: &str = "/stackable/log";
/// const LOG4J2_CONFIG_FILE: &str = "log4j2.properties";
/// const MY_PRODUCT_LOG_FILE: &str = "my-product.log4j2.xml";
/// const MAX_LOG_FILE_SIZE_IN_MIB: u32 = 10;
/// const CONSOLE_CONVERSION_PATTERN: &str = "%d{ISO8601} %-5p %m%n";
///
/// let mut cm_builder = ConfigMapBuilder::new();
/// cm_builder.metadata(ObjectMetaBuilder::default().build());
///
/// if let Some(ContainerLogConfig {
///     choice: Some(ContainerLogConfigChoice::Automatic(log_config)),
/// }) = logging.containers.get(&Container::MyProduct)
/// {
///     cm_builder.add_data(
///         LOG4J2_CONFIG_FILE,
///         product_logging::framework::create_log4j2_config(
///             &format!("{STACKABLE_LOG_DIR}/my-product"),
///             MY_PRODUCT_LOG_FILE,
///             MAX_LOG_FILE_SIZE_IN_MIB,
///             CONSOLE_CONVERSION_PATTERN,
///             log_config,
///         ),
///     );
/// }
///
/// cm_builder.build().unwrap();
/// ```
pub fn create_log4j2_config(
    log_dir: &str,
    log_file: &str,
    max_size_in_mib: u32,
    console_conversion_pattern: &str,
    config: &AutomaticContainerLogConfig,
) -> String {
    let number_of_archived_log_files = 1;

    let logger_names = config
        .loggers
        .iter()
        .filter(|(name, _)| name.as_str() != AutomaticContainerLogConfig::ROOT_LOGGER)
        .map(|(name, _)| name.escape_default().to_string())
        .collect::<Vec<String>>()
        .join(", ");
    let loggers = if logger_names.is_empty() {
        "".to_string()
    } else {
        format!("loggers = {logger_names}")
    };
    let logger_configs = config
        .loggers
        .iter()
        .filter(|(name, _)| name.as_str() != AutomaticContainerLogConfig::ROOT_LOGGER)
        .map(|(name, logger_config)| {
            format!(
                "logger.{name}.name = {name}\nlogger.{name}.level = {level}\n",
                name = name.escape_default(),
                level = logger_config.level.to_log4j_literal(),
            )
        })
        .collect::<String>();

    format!(
        r#"appenders = FILE, CONSOLE

appender.CONSOLE.type = Console
appender.CONSOLE.name = CONSOLE
appender.CONSOLE.target = SYSTEM_ERR
appender.CONSOLE.layout.type = PatternLayout
appender.CONSOLE.layout.pattern = {console_conversion_pattern}
appender.CONSOLE.filter.threshold.type = ThresholdFilter
appender.CONSOLE.filter.threshold.level = {console_log_level}

appender.FILE.type = RollingFile
appender.FILE.name = FILE
appender.FILE.fileName = {log_dir}/{log_file}
appender.FILE.filePattern = {log_dir}/{log_file}.%i
appender.FILE.layout.type = XMLLayout
appender.FILE.policies.type = Policies
appender.FILE.policies.size.type = SizeBasedTriggeringPolicy
appender.FILE.policies.size.size = {max_log_file_size_in_mib}MB
appender.FILE.strategy.type = DefaultRolloverStrategy
appender.FILE.strategy.max = {number_of_archived_log_files}
appender.FILE.filter.threshold.type = ThresholdFilter
appender.FILE.filter.threshold.level = {file_log_level}
{loggers}
{logger_configs}
rootLogger.level={root_log_level}
rootLogger.appenderRefs = CONSOLE, FILE
rootLogger.appenderRef.CONSOLE.ref = CONSOLE
rootLogger.appenderRef.FILE.ref = FILE"#,
        max_log_file_size_in_mib =
            cmp::max(1, max_size_in_mib / (1 + number_of_archived_log_files)),
        root_log_level = config.root_log_level().to_log4j2_literal(),
        console_log_level = config
            .console
            .as_ref()
            .and_then(|console| console.level)
            .unwrap_or_default()
            .to_log4j2_literal(),
        file_log_level = config
            .file
            .as_ref()
            .and_then(|file| file.level)
            .unwrap_or_default()
            .to_log4j2_literal(),
    )
}

/// Create the content of a logback XML configuration file according to the given log configuration
///
/// # Arguments
///
/// * `log_dir` - Directory where the log files are stored
/// * `log_file` - Name of the active log file; When the file is rolled over then a number is
///       appended.
/// * `max_size_in_mib` - Maximum size of all log files in MiB; This value can be slightly
///       exceeded. The value is set to 2 if the given value is lower (1 MiB for the active log
///       file and 1 MiB for the archived one).
/// * `console_conversion_pattern` - Logback conversion pattern for the console appender
/// * `config` - The logging configuration for the container
/// * `additional_config` - Optional unstructured parameter to add special cases that are not
///       covered in the logging configuration. Must adhere to the inner logback XML schema as
///       shown in the example below. It is not parsed or checked and added as is to the `logback.xml`.  
///
/// # Example
///
/// ```
/// use stackable_operator::{
///     builder::{
///         ConfigMapBuilder,
///         meta::ObjectMetaBuilder,
///     },
///     product_logging,
///     product_logging::spec::{
///         ContainerLogConfig, ContainerLogConfigChoice, Logging,
///     },
/// };
/// # use stackable_operator::{
/// #     config::fragment,
/// #     product_logging::spec::default_logging,
/// # };
/// # use strum::{Display, EnumIter};
/// #
/// # #[derive(Clone, Display, Eq, EnumIter, Ord, PartialEq, PartialOrd)]
/// # pub enum Container {
/// #     MyProduct,
/// # }
/// #
/// # let logging = fragment::validate::<Logging<Container>>(default_logging()).unwrap();
///
/// const STACKABLE_LOG_DIR: &str = "/stackable/log";
/// const LOGBACK_CONFIG_FILE: &str = "logback.xml";
/// const MY_PRODUCT_LOG_FILE: &str = "my-product.log4j.xml";
/// const MAX_LOG_FILE_SIZE_IN_MIB: u32 = 10;
/// const CONSOLE_CONVERSION_PATTERN: &str = "%d{ISO8601} %-5p %m%n";
/// const ADDITIONAL_CONFIG: &str = "  <logger name=\"foo.logger\" level=\"INFO\" additivity=\"false\"></logger>";
///
/// let mut cm_builder = ConfigMapBuilder::new();
/// cm_builder.metadata(ObjectMetaBuilder::default().build());
///
/// if let Some(ContainerLogConfig {
///     choice: Some(ContainerLogConfigChoice::Automatic(log_config)),
/// }) = logging.containers.get(&Container::MyProduct)
/// {
///     cm_builder.add_data(
///         LOGBACK_CONFIG_FILE,
///         product_logging::framework::create_logback_config(
///             &format!("{STACKABLE_LOG_DIR}/my-product"),
///             MY_PRODUCT_LOG_FILE,
///             MAX_LOG_FILE_SIZE_IN_MIB,
///             CONSOLE_CONVERSION_PATTERN,
///             log_config,
///             Some(ADDITIONAL_CONFIG)
///         ),
///     );
/// }
///
/// cm_builder.build().unwrap();
/// ```
pub fn create_logback_config(
    log_dir: &str,
    log_file: &str,
    max_size_in_mib: u32,
    console_conversion_pattern: &str,
    config: &AutomaticContainerLogConfig,
    additional_config: Option<&str>,
) -> String {
    let number_of_archived_log_files = 1;

    let loggers = config
        .loggers
        .iter()
        .filter(|(name, _)| name.as_str() != AutomaticContainerLogConfig::ROOT_LOGGER)
        .map(|(name, logger_config)| {
            format!(
                "  <logger name=\"{name}\" level=\"{level}\" />\n",
                name = name.escape_default(),
                level = logger_config.level.to_logback_literal(),
            )
        })
        .collect::<String>();

    format!(
        r#"<configuration>
  <appender name="CONSOLE" class="ch.qos.logback.core.ConsoleAppender">
    <encoder>
      <pattern>{console_conversion_pattern}</pattern>
    </encoder>
    <filter class="ch.qos.logback.classic.filter.ThresholdFilter">
      <level>{console_log_level}</level>
    </filter>
  </appender>

  <appender name="FILE" class="ch.qos.logback.core.rolling.RollingFileAppender">
    <File>{log_dir}/{log_file}</File>
    <encoder class="ch.qos.logback.core.encoder.LayoutWrappingEncoder">
      <layout class="ch.qos.logback.classic.log4j.XMLLayout" />
    </encoder>
    <filter class="ch.qos.logback.classic.filter.ThresholdFilter">
      <level>{file_log_level}</level>
    </filter>
    <rollingPolicy class="ch.qos.logback.core.rolling.FixedWindowRollingPolicy">
      <minIndex>1</minIndex>
      <maxIndex>{number_of_archived_log_files}</maxIndex>
      <FileNamePattern>{log_dir}/{log_file}.%i</FileNamePattern>
    </rollingPolicy>
    <triggeringPolicy class="ch.qos.logback.core.rolling.SizeBasedTriggeringPolicy">
      <MaxFileSize>{max_log_file_size_in_mib}MB</MaxFileSize>
    </triggeringPolicy>
  </appender>

{loggers}
{additional_config}
  <root level="{root_log_level}">
    <appender-ref ref="CONSOLE" />
    <appender-ref ref="FILE" />
  </root>
</configuration>
"#,
        max_log_file_size_in_mib =
            cmp::max(1, max_size_in_mib / (1 + number_of_archived_log_files)),
        root_log_level = config.root_log_level().to_logback_literal(),
        console_log_level = config
            .console
            .as_ref()
            .and_then(|console| console.level)
            .unwrap_or_default()
            .to_logback_literal(),
        file_log_level = config
            .file
            .as_ref()
            .and_then(|file| file.level)
            .unwrap_or_default()
            .to_logback_literal(),
        additional_config = additional_config.unwrap_or("")
    )
}

/// Create the content of a Vector configuration file according to the given log configuration
///
/// # Example
///
/// ```
/// use stackable_operator::{
///     builder::{
///         ConfigMapBuilder,
///         meta::ObjectMetaBuilder,
///     },
///     product_logging,
///     product_logging::spec::{
///         ContainerLogConfig, ContainerLogConfigChoice, Logging,
///     },
/// };
/// # use stackable_operator::{
/// #     config::fragment,
/// #     k8s_openapi::api::core::v1::Pod,
/// #     kube::runtime::reflector::ObjectRef,
/// #     product_logging::spec::default_logging,
/// #     role_utils::RoleGroupRef,
/// # };
/// # use strum::{Display, EnumIter};
/// #
/// # #[derive(Clone, Display, Eq, EnumIter, Ord, PartialEq, PartialOrd)]
/// # pub enum Container {
/// #     Vector,
/// # }
/// #
/// # let logging = fragment::validate::<Logging<Container>>(default_logging()).unwrap();
/// # let vector_aggregator_address = "vector-aggregator:6000";
/// # let role_group = RoleGroupRef {
/// #     cluster: ObjectRef::<Pod>::new("test-cluster"),
/// #     role: "role".into(),
/// #     role_group: "role-group".into(),
/// # };
///
/// let mut cm_builder = ConfigMapBuilder::new();
/// cm_builder.metadata(ObjectMetaBuilder::default().build());
///
/// let vector_log_config = if let Some(ContainerLogConfig {
///     choice: Some(ContainerLogConfigChoice::Automatic(log_config)),
/// }) = logging.containers.get(&Container::Vector)
/// {
///     Some(log_config)
/// } else {
///     None
/// };
///
/// if logging.enable_vector_agent {
///     cm_builder.add_data(
///         product_logging::framework::VECTOR_CONFIG_FILE,
///         product_logging::framework::create_vector_config(
///             &role_group,
///             vector_aggregator_address,
///             vector_log_config,
///         ),
///     );
/// }
///
/// cm_builder.build().unwrap();
/// ```
pub fn create_vector_config<T>(
    role_group: &RoleGroupRef<T>,
    vector_aggregator_address: &str,
    config: Option<&AutomaticContainerLogConfig>,
) -> String
where
    T: Resource,
{
    let vector_log_level = config
        .and_then(|config| config.file.as_ref())
        .and_then(|file| file.level)
        .unwrap_or_default();

    let vector_log_level_filter_expression = match vector_log_level {
        LogLevel::TRACE => "true",
        LogLevel::DEBUG => r#".level != "TRACE""#,
        LogLevel::INFO => r#"!includes(["TRACE", "DEBUG"], .metadata.level)"#,
        LogLevel::WARN => r#"!includes(["TRACE", "DEBUG", "INFO"], .metadata.level)"#,
        LogLevel::ERROR => r#"!includes(["TRACE", "DEBUG", "INFO", "WARN"], .metadata.level)"#,
        LogLevel::FATAL => "false",
        LogLevel::NONE => "false",
    };

    format!(
        r#"data_dir = "/stackable/vector/var"

[log_schema]
host_key = "pod"

[sources.vector]
type = "internal_logs"

[sources.files_stdout]
type = "file"
include = ["{STACKABLE_LOG_DIR}/*/*.stdout.log"]

[sources.files_stderr]
type = "file"
include = ["{STACKABLE_LOG_DIR}/*/*.stderr.log"]

[sources.files_log4j]
type = "file"
include = ["{STACKABLE_LOG_DIR}/*/*.log4j.xml"]
line_delimiter = "\r\n"

[sources.files_log4j.multiline]
mode = "halt_before"
start_pattern = "^<log4j:event"
condition_pattern = "^<log4j:event"
timeout_ms = 1000

[sources.files_log4j2]
type = "file"
include = ["{STACKABLE_LOG_DIR}/*/*.log4j2.xml"]
line_delimiter = "\r\n"

[sources.files_py]
type = "file"
include = ["{STACKABLE_LOG_DIR}/*/*.py.json"]

[sources.files_airlift]
type = "file"
include = ["{STACKABLE_LOG_DIR}/*/*.airlift.json"]

[sources.files_opa_bundle_builder]
type = "file"
include = ["{STACKABLE_LOG_DIR}/bundle-builder/current"]

[sources.files_opa_json]
type = "file"
include = ["{STACKABLE_LOG_DIR}/opa/current"]

[transforms.processed_files_opa_bundle_builder]
inputs = ["files_opa_bundle_builder"]
type = "remap"
source = '''
parsed_event = parse_regex!(strip_whitespace(strip_ansi_escape_codes(string!(.message))), r'(?P<timestamp>[0-9]{{4}}-(0[1-9]|1[0-2])-(0[1-9]|[1-2][0-9]|3[0-1])T(2[0-3]|[01][0-9]):[0-5][0-9]:[0-5][0-9].[0-9]{{6}}Z)[ ]+(?P<level>\w+)[ ]+(?P<logger>.+):[ ]+(?P<message>.*)')
.timestamp = parse_timestamp!(parsed_event.timestamp, "%Y-%m-%dT%H:%M:%S.%6fZ")
.level = parsed_event.level
.logger = parsed_event.logger
.message = parsed_event.message
'''

[transforms.processed_files_opa_json]
inputs = ["files_opa_json"]
type = "remap"
source = '''
parsed_event = parse_json!(string!(.message))
keys = keys!(parsed_event)

if includes(keys, "timestamp") {{
  .timestamp = parse_timestamp!(parsed_event.timestamp, "%Y-%m-%dT%H:%M:%S.%fZ")
}} else {{
  .timestamp = parse_timestamp!(parsed_event.time, "%Y-%m-%dT%H:%M:%SZ")
}}

if includes(keys, "decision_id") {{
  .logger = "decision"
}} else {{
  .logger = "server"
}} 

.level = upcase!(parsed_event.level)
.message = string!(parsed_event.msg)

del(parsed_event.time)
del(parsed_event.timestamp)
del(parsed_event.level)
del(parsed_event.msg)

.message = .message + "\n" + encode_key_value(object!(parsed_event), field_delimiter: "\n")
'''

[transforms.processed_files_stdout]
inputs = ["files_stdout"]
type = "remap"
source = '''
.logger = "ROOT"
.level = "INFO"
'''

[transforms.processed_files_stderr]
inputs = ["files_stderr"]
type = "remap"
source = '''
.logger = "ROOT"
.level = "ERROR"
'''

[transforms.processed_files_log4j]
inputs = ["files_log4j"]
type = "remap"
source = '''
# Wrap the event so that the log4j namespace is defined when parsing the event
wrapped_xml_event = "<root xmlns:log4j=\"http://jakarta.apache.org/log4j/\">" + string!(.message) + "</root>"
parsed_event = parse_xml(wrapped_xml_event) ?? {{ "root": {{ "event": {{ "message": .message }} }} }}
event = parsed_event.root.event

epoch_milliseconds = to_int(event.@timestamp) ?? 0
if epoch_milliseconds != 0 {{
    .timestamp = to_timestamp(epoch_milliseconds, "milliseconds") ?? null
}}
if is_null(.timestamp) {{
    .timestamp = now()
}}

.logger = to_string(event.@logger) ?? ""

.level = to_string(event.@level) ?? ""

.message = join(compact([event.message, event.throwable]), "\n") ?? .message
'''

[transforms.processed_files_log4j2]
inputs = ["files_log4j2"]
type = "remap"
source = '''
parsed_event = parse_xml!(.message).Event

.timestamp = null
instant = parsed_event.Instant
if instant != null {{
    epoch_nanoseconds = to_int(instant.@epochSecond) * 1_000_000_000 + to_int(instant.@nanoOfSecond) ?? null
    if epoch_nanoseconds != null {{
        .timestamp = to_timestamp(epoch_nanoseconds, "nanoseconds") ?? null
    }}
}}
if .timestamp == null && parsed_event.@timeMillis != null {{
    epoch_milliseconds = to_int(parsed_event.@timeMillis) ?? null
    if epoch_milliseconds != null {{
        .timestamp = to_timestamp(epoch_milliseconds, "milliseconds") ?? null
    }}
}}
if .timestamp == null {{
    .timestamp = now()
}}

.logger = parsed_event.@loggerName

.level = parsed_event.@level

exception = null
thrown = parsed_event.Thrown
if thrown != null {{
    exception = "Exception"
    thread = to_string(parsed_event.@thread) ?? null
    if thread != null {{
        exception = exception + " in thread \"" + thread + "\""
    }}
    thrown_name = to_string(thrown.@name) ?? null
    if thrown_name != null {{
        exception = exception + " " + thrown_name
    }}
    message = to_string(thrown.@localizedMessage) ??
        to_string(thrown.@message) ??
        null
    if message != null {{
        exception = exception + ": " + message
    }}
    stacktrace_items = array(thrown.ExtendedStackTrace.ExtendedStackTraceItem) ?? []
    stacktrace = ""
    for_each(stacktrace_items) -> |_index, value| {{
        stacktrace = stacktrace + "        "
        class = to_string(value.@class) ?? null
        method = to_string(value.@method) ?? null
        if class != null && method != null {{
            stacktrace = stacktrace + "at " + class + "." + method
        }}
        file = to_string(value.@file) ?? null
        line = to_string(value.@line) ?? null
        if file != null && line != null {{
            stacktrace = stacktrace + "(" + file + ":" + line + ")"
        }}
        exact = to_bool(value.@exact) ?? false
        location = to_string(value.@location) ?? null
        version = to_string(value.@version) ?? null
        if location != null && version != null {{
            stacktrace = stacktrace + " "
            if !exact {{
                stacktrace = stacktrace + "~"
            }}
            stacktrace = stacktrace + "[" + location + ":" + version + "]"
        }}
        stacktrace = stacktrace + "\n"
    }}
    if stacktrace != "" {{
        exception = exception + "\n" + stacktrace
    }}
}}
.message = join!(compact([parsed_event.Message, exception]), "\n")
'''

[transforms.processed_files_py]
inputs = ["files_py"]
type = "remap"
source = '''
parsed_event = parse_json!(.message)
.timestamp = parse_timestamp!(parsed_event.asctime, "%F %T,%3f")
.logger = parsed_event.name
if parsed_event.levelname == "DEBUG" {{
  .level = "DEBUG"
}} else if parsed_event.levelname == "INFO" {{
  .level = "INFO"
}} else if parsed_event.levelname == "WARNING" {{
  .level = "WARN"
}} else if parsed_event.levelname == "ERROR" {{
  .level = "ERROR"
}} else if parsed_event.levelname == "CRITICAL" {{
  .level = "FATAL"
}}
.message = parsed_event.message
'''

[transforms.processed_files_airlift]
inputs = ["files_airlift"]
type = "remap"
source = '''
parsed_event = parse_json!(string!(.message))
.message = join!(compact([parsed_event.message, parsed_event.stackTrace]), "\n")
.timestamp = parse_timestamp!(parsed_event.timestamp, "%Y-%m-%dT%H:%M:%S.%fZ")
.logger = parsed_event.logger
.level = parsed_event.level
.thread = parsed_event.thread
'''

[transforms.extended_logs_files]
inputs = ["processed_files_*"]
type = "remap"
source = '''
. |= parse_regex!(.file, r'^{STACKABLE_LOG_DIR}/(?P<container>.*?)/(?P<file>.*?)$')
del(.source_type)
'''

[transforms.filtered_logs_vector]
inputs = ["vector"]
type = "filter"
condition = '{vector_log_level_filter_expression}'

[transforms.extended_logs_vector]
inputs = ["filtered_logs_vector"]
type = "remap"
source = '''
.container = "vector"
.level = .metadata.level
.logger = .metadata.module_path
if exists(.file) {{ .processed_file = del(.file) }}
del(.metadata)
del(.pid)
del(.source_type)
'''

[transforms.extended_logs]
inputs = ["extended_logs_*"]
type = "remap"
source = '''
.namespace = "{namespace}"
.cluster = "{cluster_name}"
.role = "{role_name}"
.roleGroup = "{role_group_name}"
'''

[sinks.aggregator]
inputs = ["extended_logs"]
type = "vector"
address = "{vector_aggregator_address}"
"#,
        namespace = role_group.cluster.namespace.clone().unwrap_or_default(),
        cluster_name = role_group.cluster.name,
        role_name = role_group.role,
        role_group_name = role_group.role_group
    )
}

/// Create the specification of the Vector log agent container
///
/// ```
/// use stackable_operator::{
///     builder::{
///         meta::ObjectMetaBuilder,
///         PodBuilder,
///         resources::ResourceRequirementsBuilder
///     },
///     product_logging,
/// };
/// use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
/// # use stackable_operator::{
/// #     commons::product_image_selection::ResolvedProductImage,
/// #     config::fragment,
/// #     product_logging::spec::{default_logging, Logging},
/// # };
/// # use strum::{Display, EnumIter};
/// #
/// # #[derive(Clone, Display, Eq, EnumIter, Ord, PartialEq, PartialOrd)]
/// # pub enum Container {
/// #     Vector,
/// # }
/// #
/// # let logging = fragment::validate::<Logging<Container>>(default_logging()).unwrap();
///
/// # let resolved_product_image = ResolvedProductImage {
/// #     product_version: "1.0.0".into(),
/// #     app_version_label: "1.0.0".into(),
/// #     image: "docker.stackable.tech/stackable/my-product:1.0.0-stackable1.0.0".into(),
/// #     image_pull_policy: "Always".into(),
/// #     pull_secrets: None,
/// # };
///
/// let mut pod_builder = PodBuilder::new();
/// pod_builder.metadata(ObjectMetaBuilder::default().build());
///
/// let resources = ResourceRequirementsBuilder::new()
///     .with_cpu_request("1")
///     .with_cpu_limit("1")
///     .with_memory_request("1Gi")
///     .with_memory_limit("1Gi")
///     .build();
///
/// if logging.enable_vector_agent {
///     pod_builder.add_container(product_logging::framework::vector_container(
///         &resolved_product_image,
///         "config",
///         "log",
///         logging.containers.get(&Container::Vector),
///         resources,
///     ));
/// }
///
/// pod_builder.build().unwrap();
/// ```
pub fn vector_container(
    image: &ResolvedProductImage,
    config_volume_name: &str,
    log_volume_name: &str,
    log_config: Option<&ContainerLogConfig>,
    resources: ResourceRequirements,
) -> Container {
    let log_level = if let Some(ContainerLogConfig {
        choice: Some(ContainerLogConfigChoice::Automatic(automatic_log_config)),
    }) = log_config
    {
        automatic_log_config.root_log_level()
    } else {
        LogLevel::INFO
    };

    ContainerBuilder::new("vector")
        .unwrap()
        .image_from_product_image(image)
        .command(vec!["bash".into(), "-c".into()])
        .args(vec![
            format!("\
/stackable/vector/bin/vector --config {STACKABLE_CONFIG_DIR}/{VECTOR_CONFIG_FILE} & vector_pid=$! && \
if [ ! -f \"{STACKABLE_LOG_DIR}/{VECTOR_LOG_DIR}/{SHUTDOWN_FILE}\" ]; then \
mkdir -p {STACKABLE_LOG_DIR}/{VECTOR_LOG_DIR} && \
inotifywait -qq --event create {STACKABLE_LOG_DIR}/{VECTOR_LOG_DIR}; \
fi && \
kill $vector_pid"),
        ])
        .add_env_var("VECTOR_LOG", log_level.to_vector_literal())
        .add_volume_mount(config_volume_name, STACKABLE_CONFIG_DIR)
        .add_volume_mount(log_volume_name, STACKABLE_LOG_DIR)
        .resources(resources)
        .build()
}

/// Command to shut down the Vector instance
///
/// # Example
///
/// ```
/// use stackable_operator::{
///     builder::ContainerBuilder,
///     product_logging,
/// };
///
/// const STACKABLE_LOG_DIR: &str = "/stackable/log";
///
/// let args = vec![
///     "echo Perform initialization tasks ...".into(),
///     product_logging::framework::shutdown_vector_command(STACKABLE_LOG_DIR),
/// ];
///
/// let container = ContainerBuilder::new("init")
///     .unwrap()
///     .image("docker.stackable.tech/stackable/my-product:1.0.0-stackable1.0.0")
///     .command(vec!["bash".to_string(), "-c".to_string()])
///     .args(vec![args.join(" && ")])
///     .add_volume_mount("log", STACKABLE_LOG_DIR)
///     .build();
/// ```
pub fn shutdown_vector_command(stackable_log_dir: &str) -> String {
    format!(
        "mkdir -p {stackable_log_dir}/{VECTOR_LOG_DIR} && \
touch {stackable_log_dir}/{VECTOR_LOG_DIR}/{SHUTDOWN_FILE}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product_logging::spec::{AppenderConfig, LoggerConfig};
    use std::collections::BTreeMap;

    #[test]
    fn test_create_log4j2_config() {
        let log_config = AutomaticContainerLogConfig {
            loggers: vec![(
                "ROOT".to_string(),
                LoggerConfig {
                    level: LogLevel::INFO,
                },
            )]
            .into_iter()
            .collect::<BTreeMap<String, LoggerConfig>>(),
            console: Some(AppenderConfig {
                level: Some(LogLevel::TRACE),
            }),
            file: Some(AppenderConfig {
                level: Some(LogLevel::ERROR),
            }),
        };

        let log4j2_properties = create_log4j2_config(
            &format!("{STACKABLE_LOG_DIR}/my-product"),
            "my-product.log4j2.xml",
            10,
            "%d{ISO8601} %-5p %m%n",
            &log_config,
        );

        assert!(log4j2_properties.contains("appenders = FILE, CONSOLE"));
        assert!(log4j2_properties.contains("appender.CONSOLE.filter.threshold.level = TRACE"));
        assert!(log4j2_properties.contains("appender.FILE.type = RollingFile"));
        assert!(log4j2_properties.contains("appender.FILE.filter.threshold.level = ERROR"));
        assert!(!log4j2_properties.contains("loggers ="));
    }

    #[test]
    fn test_create_log4j2_config_with_additional_loggers() {
        let log_config = AutomaticContainerLogConfig {
            loggers: vec![
                (
                    "ROOT".to_string(),
                    LoggerConfig {
                        level: LogLevel::INFO,
                    },
                ),
                (
                    "test".to_string(),
                    LoggerConfig {
                        level: LogLevel::INFO,
                    },
                ),
                (
                    "test_2".to_string(),
                    LoggerConfig {
                        level: LogLevel::DEBUG,
                    },
                ),
            ]
            .into_iter()
            .collect::<BTreeMap<String, LoggerConfig>>(),
            console: Some(AppenderConfig {
                level: Some(LogLevel::TRACE),
            }),
            file: Some(AppenderConfig {
                level: Some(LogLevel::ERROR),
            }),
        };

        let log4j2_properties = create_log4j2_config(
            &format!("{STACKABLE_LOG_DIR}/my-product"),
            "my-product.log4j2.xml",
            10,
            "%d{ISO8601} %-5p %m%n",
            &log_config,
        );

        assert!(log4j2_properties.contains("appenders = FILE, CONSOLE"));
        assert!(log4j2_properties.contains("appender.CONSOLE.filter.threshold.level = TRACE"));
        assert!(log4j2_properties.contains("appender.FILE.type = RollingFile"));
        assert!(log4j2_properties.contains("appender.FILE.filter.threshold.level = ERROR"));
        assert!(log4j2_properties.contains("loggers = test, test_2"));
        assert!(log4j2_properties.contains("logger.test.level = INFO"));
        assert!(log4j2_properties.contains("logger.test_2.level = DEBUG"));
    }
}
