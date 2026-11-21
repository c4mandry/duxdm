/* PAM probe: test any service. Usage: pam_probe2 <service> [user] */
#include <security/pam_appl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int conv_fn(int n, const struct pam_message **msg,
                   struct pam_response **resp, void *data) {
    (void)data;
    *resp = calloc(n, sizeof(struct pam_response));
    if (!*resp) return PAM_CONV_ERR;
    for (int i = 0; i < n; i++) {
        if (msg[i]->msg_style == PAM_PROMPT_ECHO_OFF)
            (*resp)[i].resp = strdup("duxdmtest123");
        else if (msg[i]->msg_style == PAM_PROMPT_ECHO_ON)
            (*resp)[i].resp = strdup("duxdmtest");
    }
    return PAM_SUCCESS;
}

int main(int argc, char **argv) {
    const char *service = argc > 1 ? argv[1] : "duxdm";
    const char *user = argc > 2 ? argv[2] : "duxdmtest";
    struct pam_conv conv = { conv_fn, NULL };
    pam_handle_t *h = NULL;
    int r = pam_start(service, user, &conv, &h);
    printf("%s: start=%d (%s)\n", service, r, pam_strerror(h, r));
    if (r == PAM_SUCCESS) {
        r = pam_authenticate(h, 0);
        printf("%s: auth=%d (%s)\n", service, r, pam_strerror(h, r));
        r = pam_acct_mgmt(h, 0);
        printf("%s: acct=%d (%s)\n", service, r, pam_strerror(h, r));
        pam_end(h, r);
    }
    return 0;
}
