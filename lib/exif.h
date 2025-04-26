#ifndef __KAPY_EXIF_H__
#define __KAPY_EXIF_H__

#ifdef __cplusplus
extern "C"
{
#endif

#include <cstddef>

    typedef struct _exif_metadata_t exif_metadata_t;
    typedef struct _exif_metadata_private_t exif_metadata_private_t;

    struct _exif_metadata_t
    {
        exif_metadata_private_t *priv;
    };

    exif_metadata_t *exif_metadata_new();
    void exif_metadata_destroy(exif_metadata_t **self);

    int exif_metadata_from_blob(exif_metadata_t *self, const unsigned char *blob, size_t blob_len);
    size_t exif_metadata_to_blob(exif_metadata_t *self, unsigned char **out_blob);

    char *exif_get_tag_string(exif_metadata_t *self, const char *path);

    int exif_metadata_add_gps_info(exif_metadata_t *self, double lat, double lon, double alt);

#ifdef __cplusplus
}
#endif

#endif