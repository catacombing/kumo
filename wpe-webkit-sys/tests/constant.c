// This file was generated by gir (https://github.com/gtk-rs/gir)
// from /usr/share/gir-1.0
// from ../gir-files
// DO NOT EDIT

#include "manual.h"
#include <stdio.h>

#define PRINT_CONSTANT(CONSTANT_NAME) \
    printf("%s;", #CONSTANT_NAME); \
    printf(_Generic((CONSTANT_NAME), \
                    char *: "%s", \
                    const char *: "%s", \
                    char: "%c", \
                    signed char: "%hhd", \
                    unsigned char: "%hhu", \
                    short int: "%hd", \
                    unsigned short int: "%hu", \
                    int: "%d", \
                    unsigned int: "%u", \
                    long: "%ld", \
                    unsigned long: "%lu", \
                    long long: "%lld", \
                    unsigned long long: "%llu", \
                    float: "%f", \
                    double: "%f", \
                    long double: "%ld"), \
           CONSTANT_NAME); \
    printf("\n");

int main() {
    PRINT_CONSTANT((gint) WEBKIT_AUTHENTICATION_SCHEME_CLIENT_CERTIFICATE_PIN_REQUESTED);
    PRINT_CONSTANT((gint) WEBKIT_AUTHENTICATION_SCHEME_CLIENT_CERTIFICATE_REQUESTED);
    PRINT_CONSTANT((gint) WEBKIT_AUTHENTICATION_SCHEME_DEFAULT);
    PRINT_CONSTANT((gint) WEBKIT_AUTHENTICATION_SCHEME_HTML_FORM);
    PRINT_CONSTANT((gint) WEBKIT_AUTHENTICATION_SCHEME_HTTP_BASIC);
    PRINT_CONSTANT((gint) WEBKIT_AUTHENTICATION_SCHEME_HTTP_DIGEST);
    PRINT_CONSTANT((gint) WEBKIT_AUTHENTICATION_SCHEME_NEGOTIATE);
    PRINT_CONSTANT((gint) WEBKIT_AUTHENTICATION_SCHEME_NTLM);
    PRINT_CONSTANT((gint) WEBKIT_AUTHENTICATION_SCHEME_SERVER_TRUST_EVALUATION_REQUESTED);
    PRINT_CONSTANT((gint) WEBKIT_AUTHENTICATION_SCHEME_UNKNOWN);
    PRINT_CONSTANT((gint) WEBKIT_AUTOMATION_BROWSING_CONTEXT_PRESENTATION_TAB);
    PRINT_CONSTANT((gint) WEBKIT_AUTOMATION_BROWSING_CONTEXT_PRESENTATION_WINDOW);
    PRINT_CONSTANT((gint) WEBKIT_AUTOPLAY_ALLOW);
    PRINT_CONSTANT((gint) WEBKIT_AUTOPLAY_ALLOW_WITHOUT_SOUND);
    PRINT_CONSTANT((gint) WEBKIT_AUTOPLAY_DENY);
    PRINT_CONSTANT((gint) WEBKIT_CACHE_MODEL_DOCUMENT_BROWSER);
    PRINT_CONSTANT((gint) WEBKIT_CACHE_MODEL_DOCUMENT_VIEWER);
    PRINT_CONSTANT((gint) WEBKIT_CACHE_MODEL_WEB_BROWSER);
    PRINT_CONSTANT((gint) WEBKIT_CONTEXT_MENU_ACTION_BOLD);
    PRINT_CONSTANT((gint) WEBKIT_CONTEXT_MENU_ACTION_COPY);
    PRINT_CONSTANT((gint) WEBKIT_CONTEXT_MENU_ACTION_COPY_AUDIO_LINK_TO_CLIPBOARD);
    PRINT_CONSTANT((gint) WEBKIT_CONTEXT_MENU_ACTION_COPY_IMAGE_TO_CLIPBOARD);
    PRINT_CONSTANT((gint) WEBKIT_CONTEXT_MENU_ACTION_COPY_LINK_TO_CLIPBOARD);
    PRINT_CONSTANT((gint) WEBKIT_CONTEXT_MENU_ACTION_COPY_VIDEO_LINK_TO_CLIPBOARD);
    PRINT_CONSTANT((gint) WEBKIT_CONTEXT_MENU_ACTION_CUSTOM);
    PRINT_CONSTANT((gint) WEBKIT_CONTEXT_MENU_ACTION_CUT);
    PRINT_CONSTANT((gint) WEBKIT_CONTEXT_MENU_ACTION_DOWNLOAD_AUDIO_TO_DISK);
    PRINT_CONSTANT((gint) WEBKIT_CONTEXT_MENU_ACTION_DOWNLOAD_IMAGE_TO_DISK);
    PRINT_CONSTANT((gint) WEBKIT_CONTEXT_MENU_ACTION_DOWNLOAD_LINK_TO_DISK);
    PRINT_CONSTANT((gint) WEBKIT_CONTEXT_MENU_ACTION_DOWNLOAD_VIDEO_TO_DISK);
    PRINT_CONSTANT((gint) WEBKIT_CONTEXT_MENU_ACTION_ENTER_VIDEO_FULLSCREEN);
    PRINT_CONSTANT((gint) WEBKIT_CONTEXT_MENU_ACTION_FONT_MENU);
    PRINT_CONSTANT((gint) WEBKIT_CONTEXT_MENU_ACTION_GO_BACK);
    PRINT_CONSTANT((gint) WEBKIT_CONTEXT_MENU_ACTION_GO_FORWARD);
    PRINT_CONSTANT((gint) WEBKIT_CONTEXT_MENU_ACTION_IGNORE_GRAMMAR);
    PRINT_CONSTANT((gint) WEBKIT_CONTEXT_MENU_ACTION_IGNORE_SPELLING);
    PRINT_CONSTANT((gint) WEBKIT_CONTEXT_MENU_ACTION_INSPECT_ELEMENT);
    PRINT_CONSTANT((gint) WEBKIT_CONTEXT_MENU_ACTION_ITALIC);
    PRINT_CONSTANT((gint) WEBKIT_CONTEXT_MENU_ACTION_LEARN_SPELLING);
    PRINT_CONSTANT((gint) WEBKIT_CONTEXT_MENU_ACTION_MEDIA_MUTE);
    PRINT_CONSTANT((gint) WEBKIT_CONTEXT_MENU_ACTION_MEDIA_PAUSE);
    PRINT_CONSTANT((gint) WEBKIT_CONTEXT_MENU_ACTION_MEDIA_PLAY);
    PRINT_CONSTANT((gint) WEBKIT_CONTEXT_MENU_ACTION_NO_ACTION);
    PRINT_CONSTANT((gint) WEBKIT_CONTEXT_MENU_ACTION_NO_GUESSES_FOUND);
    PRINT_CONSTANT((gint) WEBKIT_CONTEXT_MENU_ACTION_OPEN_AUDIO_IN_NEW_WINDOW);
    PRINT_CONSTANT((gint) WEBKIT_CONTEXT_MENU_ACTION_OPEN_FRAME_IN_NEW_WINDOW);
    PRINT_CONSTANT((gint) WEBKIT_CONTEXT_MENU_ACTION_OPEN_IMAGE_IN_NEW_WINDOW);
    PRINT_CONSTANT((gint) WEBKIT_CONTEXT_MENU_ACTION_OPEN_LINK);
    PRINT_CONSTANT((gint) WEBKIT_CONTEXT_MENU_ACTION_OPEN_LINK_IN_NEW_WINDOW);
    PRINT_CONSTANT((gint) WEBKIT_CONTEXT_MENU_ACTION_OPEN_VIDEO_IN_NEW_WINDOW);
    PRINT_CONSTANT((gint) WEBKIT_CONTEXT_MENU_ACTION_OUTLINE);
    PRINT_CONSTANT((gint) WEBKIT_CONTEXT_MENU_ACTION_PASTE);
    PRINT_CONSTANT((gint) WEBKIT_CONTEXT_MENU_ACTION_RELOAD);
    PRINT_CONSTANT((gint) WEBKIT_CONTEXT_MENU_ACTION_SPELLING_GUESS);
    PRINT_CONSTANT((gint) WEBKIT_CONTEXT_MENU_ACTION_STOP);
    PRINT_CONSTANT((gint) WEBKIT_CONTEXT_MENU_ACTION_TOGGLE_MEDIA_CONTROLS);
    PRINT_CONSTANT((gint) WEBKIT_CONTEXT_MENU_ACTION_TOGGLE_MEDIA_LOOP);
    PRINT_CONSTANT((gint) WEBKIT_CONTEXT_MENU_ACTION_UNDERLINE);
    PRINT_CONSTANT((gint) WEBKIT_COOKIE_PERSISTENT_STORAGE_SQLITE);
    PRINT_CONSTANT((gint) WEBKIT_COOKIE_PERSISTENT_STORAGE_TEXT);
    PRINT_CONSTANT((gint) WEBKIT_COOKIE_POLICY_ACCEPT_ALWAYS);
    PRINT_CONSTANT((gint) WEBKIT_COOKIE_POLICY_ACCEPT_NEVER);
    PRINT_CONSTANT((gint) WEBKIT_COOKIE_POLICY_ACCEPT_NO_THIRD_PARTY);
    PRINT_CONSTANT((gint) WEBKIT_CREDENTIAL_PERSISTENCE_FOR_SESSION);
    PRINT_CONSTANT((gint) WEBKIT_CREDENTIAL_PERSISTENCE_NONE);
    PRINT_CONSTANT((gint) WEBKIT_CREDENTIAL_PERSISTENCE_PERMANENT);
    PRINT_CONSTANT((gint) WEBKIT_DOWNLOAD_ERROR_CANCELLED_BY_USER);
    PRINT_CONSTANT((gint) WEBKIT_DOWNLOAD_ERROR_DESTINATION);
    PRINT_CONSTANT((gint) WEBKIT_DOWNLOAD_ERROR_NETWORK);
    PRINT_CONSTANT(WEBKIT_EDITING_COMMAND_COPY);
    PRINT_CONSTANT(WEBKIT_EDITING_COMMAND_CREATE_LINK);
    PRINT_CONSTANT(WEBKIT_EDITING_COMMAND_CUT);
    PRINT_CONSTANT(WEBKIT_EDITING_COMMAND_INSERT_IMAGE);
    PRINT_CONSTANT(WEBKIT_EDITING_COMMAND_PASTE);
    PRINT_CONSTANT(WEBKIT_EDITING_COMMAND_PASTE_AS_PLAIN_TEXT);
    PRINT_CONSTANT(WEBKIT_EDITING_COMMAND_REDO);
    PRINT_CONSTANT(WEBKIT_EDITING_COMMAND_SELECT_ALL);
    PRINT_CONSTANT(WEBKIT_EDITING_COMMAND_UNDO);
    PRINT_CONSTANT((guint) WEBKIT_EDITOR_TYPING_ATTRIBUTE_BOLD);
    PRINT_CONSTANT((guint) WEBKIT_EDITOR_TYPING_ATTRIBUTE_ITALIC);
    PRINT_CONSTANT((guint) WEBKIT_EDITOR_TYPING_ATTRIBUTE_NONE);
    PRINT_CONSTANT((guint) WEBKIT_EDITOR_TYPING_ATTRIBUTE_STRIKETHROUGH);
    PRINT_CONSTANT((guint) WEBKIT_EDITOR_TYPING_ATTRIBUTE_UNDERLINE);
    PRINT_CONSTANT((gint) WEBKIT_FAVICON_DATABASE_ERROR_FAVICON_NOT_FOUND);
    PRINT_CONSTANT((gint) WEBKIT_FAVICON_DATABASE_ERROR_FAVICON_UNKNOWN);
    PRINT_CONSTANT((gint) WEBKIT_FAVICON_DATABASE_ERROR_NOT_INITIALIZED);
    PRINT_CONSTANT((gint) WEBKIT_FEATURE_STATUS_DEVELOPER);
    PRINT_CONSTANT((gint) WEBKIT_FEATURE_STATUS_EMBEDDER);
    PRINT_CONSTANT((gint) WEBKIT_FEATURE_STATUS_INTERNAL);
    PRINT_CONSTANT((gint) WEBKIT_FEATURE_STATUS_MATURE);
    PRINT_CONSTANT((gint) WEBKIT_FEATURE_STATUS_PREVIEW);
    PRINT_CONSTANT((gint) WEBKIT_FEATURE_STATUS_STABLE);
    PRINT_CONSTANT((gint) WEBKIT_FEATURE_STATUS_TESTABLE);
    PRINT_CONSTANT((gint) WEBKIT_FEATURE_STATUS_UNSTABLE);
    PRINT_CONSTANT((guint) WEBKIT_FIND_OPTIONS_AT_WORD_STARTS);
    PRINT_CONSTANT((guint) WEBKIT_FIND_OPTIONS_BACKWARDS);
    PRINT_CONSTANT((guint) WEBKIT_FIND_OPTIONS_CASE_INSENSITIVE);
    PRINT_CONSTANT((guint) WEBKIT_FIND_OPTIONS_NONE);
    PRINT_CONSTANT((guint) WEBKIT_FIND_OPTIONS_TREAT_MEDIAL_CAPITAL_AS_WORD_START);
    PRINT_CONSTANT((guint) WEBKIT_FIND_OPTIONS_WRAP_AROUND);
    PRINT_CONSTANT((guint) WEBKIT_HIT_TEST_RESULT_CONTEXT_DOCUMENT);
    PRINT_CONSTANT((guint) WEBKIT_HIT_TEST_RESULT_CONTEXT_EDITABLE);
    PRINT_CONSTANT((guint) WEBKIT_HIT_TEST_RESULT_CONTEXT_IMAGE);
    PRINT_CONSTANT((guint) WEBKIT_HIT_TEST_RESULT_CONTEXT_LINK);
    PRINT_CONSTANT((guint) WEBKIT_HIT_TEST_RESULT_CONTEXT_MEDIA);
    PRINT_CONSTANT((guint) WEBKIT_HIT_TEST_RESULT_CONTEXT_SCROLLBAR);
    PRINT_CONSTANT((guint) WEBKIT_HIT_TEST_RESULT_CONTEXT_SELECTION);
    PRINT_CONSTANT((guint) WEBKIT_INPUT_HINT_INHIBIT_OSK);
    PRINT_CONSTANT((guint) WEBKIT_INPUT_HINT_LOWERCASE);
    PRINT_CONSTANT((guint) WEBKIT_INPUT_HINT_NONE);
    PRINT_CONSTANT((guint) WEBKIT_INPUT_HINT_SPELLCHECK);
    PRINT_CONSTANT((guint) WEBKIT_INPUT_HINT_UPPERCASE_CHARS);
    PRINT_CONSTANT((guint) WEBKIT_INPUT_HINT_UPPERCASE_SENTENCES);
    PRINT_CONSTANT((guint) WEBKIT_INPUT_HINT_UPPERCASE_WORDS);
    PRINT_CONSTANT((gint) WEBKIT_INPUT_PURPOSE_DIGITS);
    PRINT_CONSTANT((gint) WEBKIT_INPUT_PURPOSE_EMAIL);
    PRINT_CONSTANT((gint) WEBKIT_INPUT_PURPOSE_FREE_FORM);
    PRINT_CONSTANT((gint) WEBKIT_INPUT_PURPOSE_NUMBER);
    PRINT_CONSTANT((gint) WEBKIT_INPUT_PURPOSE_PASSWORD);
    PRINT_CONSTANT((gint) WEBKIT_INPUT_PURPOSE_PHONE);
    PRINT_CONSTANT((gint) WEBKIT_INPUT_PURPOSE_URL);
    PRINT_CONSTANT((gint) WEBKIT_INSECURE_CONTENT_DISPLAYED);
    PRINT_CONSTANT((gint) WEBKIT_INSECURE_CONTENT_RUN);
    PRINT_CONSTANT((gint) WEBKIT_JAVASCRIPT_ERROR_INVALID_PARAMETER);
    PRINT_CONSTANT((gint) WEBKIT_JAVASCRIPT_ERROR_INVALID_RESULT);
    PRINT_CONSTANT((gint) WEBKIT_JAVASCRIPT_ERROR_SCRIPT_FAILED);
    PRINT_CONSTANT((gint) WEBKIT_LOAD_COMMITTED);
    PRINT_CONSTANT((gint) WEBKIT_LOAD_FINISHED);
    PRINT_CONSTANT((gint) WEBKIT_LOAD_REDIRECTED);
    PRINT_CONSTANT((gint) WEBKIT_LOAD_STARTED);
    PRINT_CONSTANT(WEBKIT_MAJOR_VERSION);
    PRINT_CONSTANT((gint) WEBKIT_MEDIA_CAPTURE_STATE_ACTIVE);
    PRINT_CONSTANT((gint) WEBKIT_MEDIA_CAPTURE_STATE_MUTED);
    PRINT_CONSTANT((gint) WEBKIT_MEDIA_CAPTURE_STATE_NONE);
    PRINT_CONSTANT((gint) WEBKIT_MEDIA_ERROR_WILL_HANDLE_LOAD);
    PRINT_CONSTANT(WEBKIT_MICRO_VERSION);
    PRINT_CONSTANT(WEBKIT_MINOR_VERSION);
    PRINT_CONSTANT((gint) WEBKIT_NAVIGATION_TYPE_BACK_FORWARD);
    PRINT_CONSTANT((gint) WEBKIT_NAVIGATION_TYPE_FORM_RESUBMITTED);
    PRINT_CONSTANT((gint) WEBKIT_NAVIGATION_TYPE_FORM_SUBMITTED);
    PRINT_CONSTANT((gint) WEBKIT_NAVIGATION_TYPE_LINK_CLICKED);
    PRINT_CONSTANT((gint) WEBKIT_NAVIGATION_TYPE_OTHER);
    PRINT_CONSTANT((gint) WEBKIT_NAVIGATION_TYPE_RELOAD);
    PRINT_CONSTANT((gint) WEBKIT_NETWORK_ERROR_CANCELLED);
    PRINT_CONSTANT((gint) WEBKIT_NETWORK_ERROR_FAILED);
    PRINT_CONSTANT((gint) WEBKIT_NETWORK_ERROR_FILE_DOES_NOT_EXIST);
    PRINT_CONSTANT((gint) WEBKIT_NETWORK_ERROR_TRANSPORT);
    PRINT_CONSTANT((gint) WEBKIT_NETWORK_ERROR_UNKNOWN_PROTOCOL);
    PRINT_CONSTANT((gint) WEBKIT_NETWORK_PROXY_MODE_CUSTOM);
    PRINT_CONSTANT((gint) WEBKIT_NETWORK_PROXY_MODE_DEFAULT);
    PRINT_CONSTANT((gint) WEBKIT_NETWORK_PROXY_MODE_NO_PROXY);
    PRINT_CONSTANT((gint) WEBKIT_PERMISSION_STATE_DENIED);
    PRINT_CONSTANT((gint) WEBKIT_PERMISSION_STATE_GRANTED);
    PRINT_CONSTANT((gint) WEBKIT_PERMISSION_STATE_PROMPT);
    PRINT_CONSTANT((gint) WEBKIT_POLICY_DECISION_TYPE_NAVIGATION_ACTION);
    PRINT_CONSTANT((gint) WEBKIT_POLICY_DECISION_TYPE_NEW_WINDOW_ACTION);
    PRINT_CONSTANT((gint) WEBKIT_POLICY_DECISION_TYPE_RESPONSE);
    PRINT_CONSTANT((gint) WEBKIT_POLICY_ERROR_CANNOT_SHOW_MIME_TYPE);
    PRINT_CONSTANT((gint) WEBKIT_POLICY_ERROR_CANNOT_SHOW_URI);
    PRINT_CONSTANT((gint) WEBKIT_POLICY_ERROR_CANNOT_USE_RESTRICTED_PORT);
    PRINT_CONSTANT((gint) WEBKIT_POLICY_ERROR_FAILED);
    PRINT_CONSTANT((gint) WEBKIT_POLICY_ERROR_FRAME_LOAD_INTERRUPTED_BY_POLICY_CHANGE);
    PRINT_CONSTANT((gint) WEBKIT_SAVE_MODE_MHTML);
    PRINT_CONSTANT((gint) WEBKIT_SCRIPT_DIALOG_ALERT);
    PRINT_CONSTANT((gint) WEBKIT_SCRIPT_DIALOG_BEFORE_UNLOAD_CONFIRM);
    PRINT_CONSTANT((gint) WEBKIT_SCRIPT_DIALOG_CONFIRM);
    PRINT_CONSTANT((gint) WEBKIT_SCRIPT_DIALOG_PROMPT);
    PRINT_CONSTANT((gint) WEBKIT_SNAPSHOT_ERROR_FAILED_TO_CREATE);
    PRINT_CONSTANT((gint) WEBKIT_TLS_ERRORS_POLICY_FAIL);
    PRINT_CONSTANT((gint) WEBKIT_TLS_ERRORS_POLICY_IGNORE);
    PRINT_CONSTANT((gint) WEBKIT_USER_CONTENT_FILTER_ERROR_INVALID_SOURCE);
    PRINT_CONSTANT((gint) WEBKIT_USER_CONTENT_FILTER_ERROR_NOT_FOUND);
    PRINT_CONSTANT((gint) WEBKIT_USER_CONTENT_INJECT_ALL_FRAMES);
    PRINT_CONSTANT((gint) WEBKIT_USER_CONTENT_INJECT_TOP_FRAME);
    PRINT_CONSTANT((gint) WEBKIT_USER_MESSAGE_UNHANDLED_MESSAGE);
    PRINT_CONSTANT((gint) WEBKIT_USER_SCRIPT_INJECT_AT_DOCUMENT_END);
    PRINT_CONSTANT((gint) WEBKIT_USER_SCRIPT_INJECT_AT_DOCUMENT_START);
    PRINT_CONSTANT((gint) WEBKIT_USER_STYLE_LEVEL_AUTHOR);
    PRINT_CONSTANT((gint) WEBKIT_USER_STYLE_LEVEL_USER);
    PRINT_CONSTANT((guint) WEBKIT_WEBSITE_DATA_ALL);
    PRINT_CONSTANT((guint) WEBKIT_WEBSITE_DATA_COOKIES);
    PRINT_CONSTANT((guint) WEBKIT_WEBSITE_DATA_DEVICE_ID_HASH_SALT);
    PRINT_CONSTANT((guint) WEBKIT_WEBSITE_DATA_DISK_CACHE);
    PRINT_CONSTANT((guint) WEBKIT_WEBSITE_DATA_DOM_CACHE);
    PRINT_CONSTANT((guint) WEBKIT_WEBSITE_DATA_HSTS_CACHE);
    PRINT_CONSTANT((guint) WEBKIT_WEBSITE_DATA_INDEXEDDB_DATABASES);
    PRINT_CONSTANT((guint) WEBKIT_WEBSITE_DATA_ITP);
    PRINT_CONSTANT((guint) WEBKIT_WEBSITE_DATA_LOCAL_STORAGE);
    PRINT_CONSTANT((guint) WEBKIT_WEBSITE_DATA_MEMORY_CACHE);
    PRINT_CONSTANT((guint) WEBKIT_WEBSITE_DATA_OFFLINE_APPLICATION_CACHE);
    PRINT_CONSTANT((guint) WEBKIT_WEBSITE_DATA_SERVICE_WORKER_REGISTRATIONS);
    PRINT_CONSTANT((guint) WEBKIT_WEBSITE_DATA_SESSION_STORAGE);
    PRINT_CONSTANT((gint) WEBKIT_WEB_EXTENSION_MATCH_PATTERN_ERROR_INVALID_HOST);
    PRINT_CONSTANT((gint) WEBKIT_WEB_EXTENSION_MATCH_PATTERN_ERROR_INVALID_PATH);
    PRINT_CONSTANT((gint) WEBKIT_WEB_EXTENSION_MATCH_PATTERN_ERROR_INVALID_SCHEME);
    PRINT_CONSTANT((gint) WEBKIT_WEB_EXTENSION_MATCH_PATTERN_ERROR_UNKNOWN);
    PRINT_CONSTANT((gint) WEBKIT_WEB_EXTENSION_MODE_MANIFESTV2);
    PRINT_CONSTANT((gint) WEBKIT_WEB_EXTENSION_MODE_MANIFESTV3);
    PRINT_CONSTANT((gint) WEBKIT_WEB_EXTENSION_MODE_NONE);
    PRINT_CONSTANT((gint) WEBKIT_WEB_PROCESS_CRASHED);
    PRINT_CONSTANT((gint) WEBKIT_WEB_PROCESS_EXCEEDED_MEMORY_LIMIT);
    PRINT_CONSTANT((gint) WEBKIT_WEB_PROCESS_TERMINATED_BY_API);
    return 0;
}
