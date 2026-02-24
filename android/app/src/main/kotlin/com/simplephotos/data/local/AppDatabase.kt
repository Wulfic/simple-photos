package com.simplephotos.data.local

import androidx.room.Database
import androidx.room.RoomDatabase
import com.simplephotos.data.local.dao.AlbumDao
import com.simplephotos.data.local.dao.BackupFolderDao
import com.simplephotos.data.local.dao.BlobQueueDao
import com.simplephotos.data.local.dao.PhotoDao
import com.simplephotos.data.local.entities.*

@Database(
    entities = [PhotoEntity::class, AlbumEntity::class, PhotoAlbumXRef::class, BlobQueueEntity::class, BackupFolderEntity::class],
    version = 4,
    exportSchema = false
)
abstract class AppDatabase : RoomDatabase() {
    abstract fun photoDao(): PhotoDao
    abstract fun albumDao(): AlbumDao
    abstract fun blobQueueDao(): BlobQueueDao
    abstract fun backupFolderDao(): BackupFolderDao
}
